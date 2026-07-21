//! `SQLite` implementation for Board-owned coordination state.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::Path;
use std::time::Duration;

use directories::ProjectDirs;
use gareji_board_domain::{
    ActiveWorkAssessment, ActivityTimeline, AgentLoopExecutionTarget, AgentPlan,
    AgentPlanUpdateReceipt, AgentPlanUpdateRequest, AgentProfileSaveReceipt,
    AgentProfileSaveRequest, AgentProfileSummary, ApprovalRequirement, AttachmentReceipt,
    AttachmentRequest, AttachmentTarget, BlueprintApplication, BlueprintApplicationReceipt,
    BlueprintApproachNotePin, BlueprintLink, BlueprintLinkKind, BlueprintNode, BlueprintNodeKind,
    BlueprintRuntimeBinding, BlueprintScope, CheckpointAttachment, CheckpointReconciliation,
    ControlGraphRevision, ControlNode, ControlNodeKind, ControlRoute, ControlSignal,
    ExecutionWorkspaceConnection, ExecutionWorkspaceKind, ExecutionWorkspaceSaveReceipt,
    ExecutionWorkspaceSaveRequest, GraphAnchor, GraphCanvasLayout, GraphEntry,
    GraphRewriteDecision, GraphRewriteDecisionReceipt, GraphRewriteDecisionRequest,
    GraphRewriteOperation, GraphRewriteProposal, GraphRewriteProposalStatus, NoteSocketKind,
    OrchestrationBlueprintRevision, PortfolioNode, PortfolioNodeKind,
    PortfolioOrchestrationRevision, PortfolioPostAction, PortfolioProjectSelector, PortfolioRoute,
    PortfolioRun, PortfolioRunStatus, PortfolioRunStep, PortfolioSchedule,
    PortfolioScheduleControl, PortfolioScheduleControlSaveReceipt,
    PortfolioScheduleControlSaveRequest, PortfolioSignal, PortfolioSnapshot, ProjectCreateReceipt,
    ProjectCreateRequest, ProjectGraphBinding, ProjectGraphBindingSaveReceipt,
    ProjectGraphBindingSaveRequest, ProjectHealth, ProjectSummary, ReconciliationDecision,
    ReconciliationReceipt, ReconciliationRequest, RouteDecision, RouteDecisionReceipt,
    RouteDecisionRequest, WorkItemCounts, WorkItemCreateReceipt, WorkItemCreateRequest,
    WorkItemGraphPosition, WorkItemState, WorkItemSummary, WorkItemTransitionReceipt,
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
                 CREATE TABLE IF NOT EXISTS board_agent_profiles (
                   id TEXT PRIMARY KEY,
                   role TEXT NOT NULL,
                   instruction_ref TEXT
                 );
                 CREATE TABLE IF NOT EXISTS board_agent_profile_capabilities (
                   agent_profile_id TEXT NOT NULL,
                   capability TEXT NOT NULL,
                   PRIMARY KEY (agent_profile_id, capability),
                   FOREIGN KEY (agent_profile_id) REFERENCES board_agent_profiles(id) ON DELETE CASCADE
                 );
                 CREATE TABLE IF NOT EXISTS board_agent_profile_skills (
                   agent_profile_id TEXT NOT NULL,
                   skill_ref TEXT NOT NULL,
                   PRIMARY KEY (agent_profile_id, skill_ref),
                   FOREIGN KEY (agent_profile_id) REFERENCES board_agent_profiles(id) ON DELETE CASCADE
                 );
                 CREATE TABLE IF NOT EXISTS board_work_items (
                   id TEXT PRIMARY KEY,
                   project_id TEXT NOT NULL,
                   title TEXT NOT NULL,
                   priority INTEGER NOT NULL DEFAULT 100 CHECK (priority > 0),
                   approval_requirement TEXT NOT NULL DEFAULT 'none' CHECK (
                     approval_requirement IN ('none', 'explicit')
                   ),
                   agent_profile_id TEXT,
                   state TEXT NOT NULL CHECK (
                     state IN ('backlog', 'todo', 'in_progress', 'in_review', 'blocked', 'done', 'cancelled')
                   ),
                   FOREIGN KEY (project_id) REFERENCES board_projects(id),
                   FOREIGN KEY (agent_profile_id) REFERENCES board_agent_profiles(id)
                 );
                 CREATE INDEX IF NOT EXISTS board_work_items_by_project_state
                   ON board_work_items(project_id, state);
                 CREATE TABLE IF NOT EXISTS board_work_item_dependencies (
                   work_item_id TEXT NOT NULL,
                   dependency_work_item_id TEXT NOT NULL,
                   PRIMARY KEY (work_item_id, dependency_work_item_id),
                   CHECK (work_item_id <> dependency_work_item_id),
                   FOREIGN KEY (work_item_id) REFERENCES board_work_items(id) ON DELETE RESTRICT,
                   FOREIGN KEY (dependency_work_item_id) REFERENCES board_work_items(id) ON DELETE RESTRICT
                 );
                 CREATE INDEX IF NOT EXISTS board_dependencies_by_prerequisite
                   ON board_work_item_dependencies(dependency_work_item_id, work_item_id);
                 CREATE TABLE IF NOT EXISTS board_work_item_required_capabilities (
                   work_item_id TEXT NOT NULL,
                   capability TEXT NOT NULL,
                   PRIMARY KEY (work_item_id, capability),
                   FOREIGN KEY (work_item_id) REFERENCES board_work_items(id) ON DELETE CASCADE
                 );
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
        create_auxiliary_storage(&connection)?;
        migrate_agent_profile_columns(&connection)?;
        migrate_work_item_columns(&connection)?;
        set_schema_version(&connection)?;
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
        insert_sample_projects(&transaction)?;
        insert_sample_execution_workspaces(&transaction)?;
        insert_sample_agent_profiles(&transaction)?;
        insert_sample_work_items(&transaction)?;
        transaction.commit().map_err(StoreError::Sqlite)?;
        Ok(true)
    }

    /// Replace the bundled portfolio names with explicit disposable-demo labels.
    ///
    /// This is intentionally separate from seeding so normal first-run Board
    /// workspaces retain their product names. Only the isolated demo launcher
    /// should call it.
    ///
    /// # Errors
    ///
    /// Returns a storage error when a bundled demo project is missing or the
    /// update transaction cannot complete.
    pub fn apply_demo_project_labels(&mut self) -> Result<(), StoreError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StoreError::Sqlite)?;
        for (project_id, name) in [
            ("gareji-board", "Gareji Board · Sample"),
            ("gareji-core", "Gareji Core · Sample"),
            ("zettelkasten-plugin", "Sample Knowledge Plugin"),
        ] {
            let changed = transaction
                .execute(
                    "UPDATE board_projects SET name = ?2 WHERE id = ?1",
                    params![project_id, name],
                )
                .map_err(StoreError::Sqlite)?;
            if changed != 1 {
                return Err(StoreError::CorruptState("demo project is missing"));
            }
        }
        transaction.commit().map_err(StoreError::Sqlite)
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

    /// Atomically add one Board project with its first local Execution workspace.
    ///
    /// # Errors
    ///
    /// Returns a bounded validation, duplicate-project, storage, or corrupt-state error.
    /// A rejected request leaves neither a project nor a workspace connection behind.
    pub fn create_project(
        &mut self,
        request: &ProjectCreateRequest,
    ) -> Result<ProjectCreateReceipt, StoreError> {
        validate_stable_identifier(&request.id)?;
        validate_title(&request.name)?;
        if request.name.trim() != request.name || request.execution_cap == 0 {
            return Err(StoreError::InvalidRequest);
        }
        let execution_workspace = canonical_execution_workspace(&request.execution_workspace)?;
        if execution_workspace.project_id != request.id
            || execution_workspace.kind != ExecutionWorkspaceKind::LocalDirectory
        {
            return Err(StoreError::InvalidRequest);
        }

        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StoreError::Sqlite)?;
        let project_exists: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM board_projects WHERE id = ?1)",
                [&request.id],
                |row| row.get(0),
            )
            .map_err(StoreError::Sqlite)?;
        if project_exists {
            return Err(StoreError::ProjectAlreadyExists);
        }
        transaction
            .execute(
                "INSERT INTO board_projects (id, name, health, execution_cap)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    request.id,
                    request.name,
                    ProjectHealth::Idle.as_str(),
                    request.execution_cap
                ],
            )
            .map_err(StoreError::Sqlite)?;
        transaction
            .execute(
                "INSERT INTO board_execution_workspaces (project_id, kind, location)
                 VALUES (?1, ?2, ?3)",
                params![
                    execution_workspace.project_id,
                    execution_workspace.kind.as_str(),
                    execution_workspace.location
                ],
            )
            .map_err(StoreError::Sqlite)?;
        transaction.commit().map_err(StoreError::Sqlite)?;

        Ok(ProjectCreateReceipt {
            project: ProjectSummary {
                id: request.id.clone(),
                name: request.name.clone(),
                health: ProjectHealth::Idle,
                execution_cap: request.execution_cap,
                work_items: WorkItemCounts::default(),
            },
            execution_workspace,
        })
    }

    /// Persist one immutable Control graph revision.
    ///
    /// Returns `true` when inserted and `false` for an identical retry.
    ///
    /// # Errors
    ///
    /// Returns a bounded validation, conflicting-revision, serialization, or storage error.
    pub fn save_control_graph_revision(
        &mut self,
        graph: &ControlGraphRevision,
    ) -> Result<bool, StoreError> {
        graph.validate().map_err(|_| StoreError::InvalidRequest)?;
        let definition = serde_json::to_string(graph).map_err(StoreError::Json)?;
        let inserted = self
            .connection
            .execute(
                "INSERT OR IGNORE INTO board_control_graph_revisions
                   (graph_id, revision_id, definition_json)
                 VALUES (?1, ?2, ?3)",
                params![graph.graph_id, graph.revision_id, definition],
            )
            .map_err(StoreError::Sqlite)?;
        if inserted == 1 {
            return Ok(true);
        }
        let existing: String = self
            .connection
            .query_row(
                "SELECT definition_json
                 FROM board_control_graph_revisions
                 WHERE graph_id = ?1 AND revision_id = ?2",
                params![graph.graph_id, graph.revision_id],
                |row| row.get(0),
            )
            .map_err(StoreError::Sqlite)?;
        if existing == definition {
            Ok(false)
        } else {
            Err(StoreError::GraphRevisionAlreadyExists)
        }
    }

    /// Ensure the small built-in graph catalog is available for project selection.
    ///
    /// Returns the number of newly inserted immutable revisions.
    ///
    /// # Errors
    ///
    /// Returns a validation, conflicting-revision, serialization, or storage error.
    pub fn ensure_builtin_control_graphs(&mut self) -> Result<usize, StoreError> {
        let mut inserted = 0;
        for graph in builtin_control_graphs() {
            inserted += usize::from(self.save_control_graph_revision(&graph)?);
        }
        Ok(inserted)
    }

    /// Load every immutable Control graph revision in stable identity order.
    ///
    /// # Errors
    ///
    /// Returns a storage or corrupt-state error.
    pub fn load_control_graph_revisions(&self) -> Result<Vec<ControlGraphRevision>, StoreError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT definition_json
                 FROM board_control_graph_revisions
                 ORDER BY graph_id, revision_id",
            )
            .map_err(StoreError::Sqlite)?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(StoreError::Sqlite)?;
        let mut graphs = Vec::new();
        for row in rows {
            let definition = row.map_err(StoreError::Sqlite)?;
            let graph: ControlGraphRevision =
                serde_json::from_str(&definition).map_err(StoreError::Json)?;
            graph
                .validate()
                .map_err(|_| StoreError::CorruptState("invalid Control graph revision"))?;
            graphs.push(graph);
        }
        Ok(graphs)
    }

    /// Load every Board-local Graph canvas layout in stable Graph identity order.
    ///
    /// # Errors
    ///
    /// Returns a storage, serialization, or corrupt-state error.
    pub fn load_graph_canvas_layouts(&self) -> Result<Vec<GraphCanvasLayout>, StoreError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT layout_json
                 FROM board_graph_canvas_layouts
                 ORDER BY graph_id",
            )
            .map_err(StoreError::Sqlite)?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(StoreError::Sqlite)?;
        let mut layouts = Vec::new();
        for row in rows {
            let layout: GraphCanvasLayout = serde_json::from_str(&row.map_err(StoreError::Sqlite)?)
                .map_err(StoreError::Json)?;
            layout
                .validate()
                .map_err(|_| StoreError::CorruptState("invalid Graph canvas layout"))?;
            layouts.push(layout);
        }
        Ok(layouts)
    }

    /// Save presentation-only node positions without changing a Graph revision.
    ///
    /// # Errors
    ///
    /// Returns a validation, serialization, or storage error.
    pub fn save_graph_canvas_layout(
        &mut self,
        layout: &GraphCanvasLayout,
    ) -> Result<(), StoreError> {
        layout.validate().map_err(|_| StoreError::InvalidRequest)?;
        let layout_json = serde_json::to_string(layout).map_err(StoreError::Json)?;
        self.connection
            .execute(
                "INSERT INTO board_graph_canvas_layouts (graph_id, layout_json, updated_at)
                 VALUES (?1, ?2, CURRENT_TIMESTAMP)
                 ON CONFLICT(graph_id) DO UPDATE SET
                   layout_json = excluded.layout_json,
                   updated_at = CURRENT_TIMESTAMP",
                params![layout.graph_id, layout_json],
            )
            .map_err(StoreError::Sqlite)?;
        Ok(())
    }

    /// Persist one immutable, project-independent Orchestration Blueprint revision.
    ///
    /// Returns `true` when inserted and `false` for an identical retry.
    ///
    /// # Errors
    ///
    /// Returns a validation, conflicting-revision, serialization, or storage error.
    pub fn save_orchestration_blueprint_revision(
        &mut self,
        revision: &OrchestrationBlueprintRevision,
    ) -> Result<bool, StoreError> {
        revision
            .validate()
            .map_err(|_| StoreError::InvalidRequest)?;
        let definition = serde_json::to_string(revision).map_err(StoreError::Json)?;
        let inserted = self
            .connection
            .execute(
                "INSERT OR IGNORE INTO board_orchestration_blueprint_revisions
                   (blueprint_id, revision_id, definition_json)
                 VALUES (?1, ?2, ?3)",
                params![revision.blueprint_id, revision.revision_id, definition],
            )
            .map_err(StoreError::Sqlite)?;
        if inserted == 1 {
            return Ok(true);
        }
        let existing: String = self
            .connection
            .query_row(
                "SELECT definition_json
                 FROM board_orchestration_blueprint_revisions
                 WHERE blueprint_id = ?1 AND revision_id = ?2",
                params![revision.blueprint_id, revision.revision_id],
                |row| row.get(0),
            )
            .map_err(StoreError::Sqlite)?;
        if existing == definition {
            Ok(false)
        } else {
            Err(StoreError::BlueprintRevisionAlreadyExists)
        }
    }

    /// Ensure the portable built-in Blueprint catalog is available idempotently.
    ///
    /// # Errors
    ///
    /// Returns a validation, conflicting-revision, serialization, or storage error.
    pub fn ensure_builtin_orchestration_blueprints(&mut self) -> Result<usize, StoreError> {
        let mut inserted = 0;
        for revision in builtin_orchestration_blueprints() {
            inserted += usize::from(self.save_orchestration_blueprint_revision(&revision)?);
        }
        Ok(inserted)
    }

    /// Load every immutable Orchestration Blueprint revision in stable order.
    ///
    /// # Errors
    ///
    /// Returns a storage or corrupt-state error.
    pub fn load_orchestration_blueprint_revisions(
        &self,
    ) -> Result<Vec<OrchestrationBlueprintRevision>, StoreError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT definition_json
                 FROM board_orchestration_blueprint_revisions
                 ORDER BY blueprint_id, revision_id",
            )
            .map_err(StoreError::Sqlite)?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(StoreError::Sqlite)?;
        let mut revisions = Vec::new();
        for row in rows {
            let definition = row.map_err(StoreError::Sqlite)?;
            let revision: OrchestrationBlueprintRevision =
                serde_json::from_str(&definition).map_err(StoreError::Json)?;
            revision.validate().map_err(|_| {
                StoreError::CorruptState("invalid Orchestration Blueprint revision")
            })?;
            revisions.push(revision);
        }
        Ok(revisions)
    }

    /// Atomically record one accepted Blueprint Application and its concrete binding.
    ///
    /// This records Board intent only. It does not start a Runner or change Work item state.
    /// Identical retries are idempotent.
    ///
    /// # Errors
    ///
    /// Returns a validation, stale-preview, conflicting-identity, or storage error.
    pub fn accept_blueprint_application(
        &mut self,
        application: &BlueprintApplication,
        runtime_binding: &BlueprintRuntimeBinding,
    ) -> Result<BlueprintApplicationReceipt, StoreError> {
        validate_blueprint_application(application, runtime_binding)?;
        let workspace = canonical_execution_workspace(&runtime_binding.execution_workspace)?;
        let pins = canonical_blueprint_note_pins(&runtime_binding.approach_notes)?;
        let pins_json = serde_json::to_string(&pins).map_err(StoreError::Json)?;

        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StoreError::Sqlite)?;
        let expected = BlueprintApplicationReceipt {
            application: application.clone(),
            runtime_binding: BlueprintRuntimeBinding {
                execution_workspace: workspace.clone(),
                approach_notes: pins.clone(),
                ..runtime_binding.clone()
            },
            created: false,
        };
        if let Some(existing) =
            load_blueprint_application_receipt(&transaction, &application.application_id)?
        {
            if existing != expected {
                return Err(StoreError::BlueprintApplicationAlreadyExists);
            }
            transaction.commit().map_err(StoreError::Sqlite)?;
            return Ok(existing);
        }
        validate_blueprint_application_facts(
            &transaction,
            application,
            runtime_binding,
            &workspace,
            &pins,
        )?;

        let application_inserted = transaction
            .execute(
                "INSERT OR IGNORE INTO board_blueprint_applications
                   (application_id, blueprint_id, revision_id, entry_node_id, project_id, work_item_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    application.application_id,
                    application.blueprint_id,
                    application.revision_id,
                    application.entry_node_id,
                    application.project_id,
                    application.work_item_id,
                ],
            )
            .map_err(StoreError::Sqlite)?;
        let binding_inserted = transaction
            .execute(
                "INSERT OR IGNORE INTO board_blueprint_runtime_bindings
                   (application_id, project_id, work_item_id, agent_profile_id,
                    execution_workspace_kind, execution_workspace_location, approach_notes_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    runtime_binding.application_id,
                    runtime_binding.project_id,
                    runtime_binding.work_item_id,
                    runtime_binding.agent_profile_id,
                    workspace.kind.as_str(),
                    workspace.location,
                    pins_json,
                ],
            )
            .map_err(StoreError::Sqlite)?;

        if application_inserted != 1 || binding_inserted != 1 {
            return Err(StoreError::ConcurrentChange);
        }
        transaction.commit().map_err(StoreError::Sqlite)?;
        Ok(BlueprintApplicationReceipt {
            application: application.clone(),
            runtime_binding: BlueprintRuntimeBinding {
                execution_workspace: workspace,
                approach_notes: pins,
                ..runtime_binding.clone()
            },
            created: true,
        })
    }

    /// Load accepted Blueprint Applications and pinned Runtime Bindings in stable order.
    ///
    /// # Errors
    ///
    /// Returns a storage or corrupt-state error.
    pub fn load_blueprint_applications(
        &self,
    ) -> Result<Vec<BlueprintApplicationReceipt>, StoreError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT application_id
                 FROM board_blueprint_applications
                 ORDER BY created_at, application_id",
            )
            .map_err(StoreError::Sqlite)?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(StoreError::Sqlite)?;
        let mut applications = Vec::new();
        for row in rows {
            let application_id = row.map_err(StoreError::Sqlite)?;
            applications.push(
                load_blueprint_application_receipt(&self.connection, &application_id)?.ok_or(
                    StoreError::CorruptState("Blueprint Application lost its Runtime Binding"),
                )?,
            );
        }
        Ok(applications)
    }

    /// Persist one immutable portfolio orchestration revision.
    ///
    /// Returns `true` when inserted and `false` for an identical retry.
    ///
    /// # Errors
    ///
    /// Returns a bounded validation, conflicting-revision, serialization, or storage error.
    pub fn save_portfolio_orchestration_revision(
        &mut self,
        revision: &PortfolioOrchestrationRevision,
    ) -> Result<bool, StoreError> {
        revision
            .validate()
            .map_err(|_| StoreError::InvalidRequest)?;
        let definition = serde_json::to_string(revision).map_err(StoreError::Json)?;
        let inserted = self
            .connection
            .execute(
                "INSERT OR IGNORE INTO board_portfolio_orchestration_revisions
                   (orchestration_id, revision_id, definition_json)
                 VALUES (?1, ?2, ?3)",
                params![revision.orchestration_id, revision.revision_id, definition],
            )
            .map_err(StoreError::Sqlite)?;
        if inserted == 1 {
            return Ok(true);
        }
        let existing: String = self
            .connection
            .query_row(
                "SELECT definition_json
                 FROM board_portfolio_orchestration_revisions
                 WHERE orchestration_id = ?1 AND revision_id = ?2",
                params![revision.orchestration_id, revision.revision_id],
                |row| row.get(0),
            )
            .map_err(StoreError::Sqlite)?;
        if existing == definition {
            Ok(false)
        } else {
            Err(StoreError::PortfolioOrchestrationRevisionAlreadyExists)
        }
    }

    /// Ensure the built-in portfolio orchestration is available idempotently.
    ///
    /// # Errors
    ///
    /// Returns a validation, conflicting-revision, serialization, or storage error.
    pub fn ensure_builtin_portfolio_orchestrations(&mut self) -> Result<usize, StoreError> {
        let mut inserted = 0;
        for revision in builtin_portfolio_orchestrations() {
            inserted += usize::from(self.save_portfolio_orchestration_revision(&revision)?);
        }
        Ok(inserted)
    }

    /// Load every immutable portfolio orchestration revision in stable order.
    ///
    /// # Errors
    ///
    /// Returns a storage or corrupt-state error.
    pub fn load_portfolio_orchestration_revisions(
        &self,
    ) -> Result<Vec<PortfolioOrchestrationRevision>, StoreError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT definition_json
                 FROM board_portfolio_orchestration_revisions
                 ORDER BY orchestration_id, revision_id",
            )
            .map_err(StoreError::Sqlite)?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(StoreError::Sqlite)?;
        let mut revisions = Vec::new();
        for row in rows {
            let definition = row.map_err(StoreError::Sqlite)?;
            let revision: PortfolioOrchestrationRevision =
                serde_json::from_str(&definition).map_err(StoreError::Json)?;
            revision.validate().map_err(|_| {
                StoreError::CorruptState("invalid Portfolio orchestration revision")
            })?;
            revisions.push(revision);
        }
        Ok(revisions)
    }

    /// Load the most recently created Run for one immutable Portfolio revision.
    ///
    /// # Errors
    ///
    /// Returns a storage or corrupt-state error.
    pub fn load_latest_portfolio_run(
        &self,
        orchestration_id: &str,
        revision_id: &str,
    ) -> Result<Option<PortfolioRun>, StoreError> {
        self.connection
            .query_row(
                "SELECT run_id, orchestration_id, revision_id, current_node_id,
                        status, completed_steps, next_tick_at_epoch_seconds
                 FROM board_portfolio_runs
                 WHERE orchestration_id = ?1 AND revision_id = ?2
                 ORDER BY rowid DESC
                 LIMIT 1",
                params![orchestration_id, revision_id],
                portfolio_run_from_row,
            )
            .optional()
            .map_err(StoreError::Sqlite)?
            .map_or(Ok(None), |run| run.map(Some))
    }

    /// Load the effective automatic-tick control for one Portfolio revision.
    /// Missing runtime state derives from the immutable revision schedule.
    ///
    /// # Errors
    ///
    /// Returns a storage error.
    pub fn load_portfolio_schedule_control(
        &self,
        revision: &PortfolioOrchestrationRevision,
    ) -> Result<PortfolioScheduleControl, StoreError> {
        self.connection
            .query_row(
                "SELECT automatic_ticks_enabled
                 FROM board_portfolio_schedule_controls
                 WHERE orchestration_id = ?1 AND revision_id = ?2",
                params![revision.orchestration_id, revision.revision_id],
                |row| {
                    Ok(PortfolioScheduleControl {
                        orchestration_id: revision.orchestration_id.clone(),
                        revision_id: revision.revision_id.clone(),
                        automatic_ticks_enabled: row.get(0)?,
                    })
                },
            )
            .optional()
            .map_err(StoreError::Sqlite)
            .map(|stored| {
                stored.unwrap_or_else(|| PortfolioScheduleControl::from_revision(revision))
            })
    }

    /// Optimistically pause or resume automatic ticks without changing a Run.
    ///
    /// # Errors
    ///
    /// Returns a validation, concurrent-change, relationship, or storage error.
    pub fn save_portfolio_schedule_control(
        &mut self,
        request: &PortfolioScheduleControlSaveRequest,
    ) -> Result<PortfolioScheduleControlSaveReceipt, StoreError> {
        if request.expected.orchestration_id != request.target.orchestration_id
            || request.expected.revision_id != request.target.revision_id
        {
            return Err(StoreError::InvalidRequest);
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StoreError::Sqlite)?;
        let definition: Option<String> = transaction
            .query_row(
                "SELECT definition_json
                 FROM board_portfolio_orchestration_revisions
                 WHERE orchestration_id = ?1 AND revision_id = ?2",
                params![request.target.orchestration_id, request.target.revision_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(StoreError::Sqlite)?;
        let Some(definition) = definition else {
            return Err(StoreError::InvalidRequest);
        };
        let revision: PortfolioOrchestrationRevision =
            serde_json::from_str(&definition).map_err(StoreError::Json)?;
        if request.target.automatic_ticks_enabled && revision.schedule.interval_seconds().is_none()
        {
            return Err(StoreError::InvalidRequest);
        }
        let stored: Option<bool> = transaction
            .query_row(
                "SELECT automatic_ticks_enabled
                 FROM board_portfolio_schedule_controls
                 WHERE orchestration_id = ?1 AND revision_id = ?2",
                params![request.target.orchestration_id, request.target.revision_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(StoreError::Sqlite)?;
        let previous = stored.map_or_else(
            || PortfolioScheduleControl::from_revision(&revision),
            |automatic_ticks_enabled| PortfolioScheduleControl {
                orchestration_id: request.target.orchestration_id.clone(),
                revision_id: request.target.revision_id.clone(),
                automatic_ticks_enabled,
            },
        );
        if previous != request.expected {
            return Err(StoreError::ConcurrentChange);
        }
        let changed = previous != request.target;
        if changed {
            transaction
                .execute(
                    "INSERT INTO board_portfolio_schedule_controls
                       (orchestration_id, revision_id, automatic_ticks_enabled)
                     VALUES (?1, ?2, ?3)
                     ON CONFLICT(orchestration_id, revision_id) DO UPDATE SET
                       automatic_ticks_enabled = excluded.automatic_ticks_enabled,
                       updated_at = CURRENT_TIMESTAMP",
                    params![
                        request.target.orchestration_id,
                        request.target.revision_id,
                        request.target.automatic_ticks_enabled,
                    ],
                )
                .map_err(StoreError::Sqlite)?;
        }
        transaction.commit().map_err(StoreError::Sqlite)?;
        Ok(PortfolioScheduleControlSaveReceipt {
            previous,
            resulting: request.target.clone(),
            changed,
        })
    }

    /// Load every append-only step for one Portfolio Run.
    ///
    /// # Errors
    ///
    /// Returns a storage or corrupt-state error.
    pub fn load_portfolio_run_steps(
        &self,
        run_id: &str,
    ) -> Result<Vec<PortfolioRunStep>, StoreError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT run_id, sequence, node_id, signal, destination_node_id,
                        selected_project_id, recorded_at_epoch_seconds
                 FROM board_portfolio_run_steps
                 WHERE run_id = ?1
                 ORDER BY sequence",
            )
            .map_err(StoreError::Sqlite)?;
        let rows = statement
            .query_map([run_id], portfolio_run_step_from_row)
            .map_err(StoreError::Sqlite)?;
        let mut steps = Vec::new();
        for row in rows {
            steps.push(row.map_err(StoreError::Sqlite)??);
        }
        Ok(steps)
    }

    /// Atomically persist one optimistic Portfolio Run transition and its step.
    ///
    /// `observed_latest` is the latest Run the caller used when deciding the tick.
    /// Comparing it inside the immediate transaction prevents the desktop and an
    /// external scheduler from both advancing, or both restarting, one revision.
    ///
    /// # Errors
    ///
    /// Returns a concurrent-change, relationship, or storage error.
    pub fn record_portfolio_tick(
        &mut self,
        observed_latest: Option<&PortfolioRun>,
        resulting: &PortfolioRun,
        step: &PortfolioRunStep,
    ) -> Result<(), StoreError> {
        if step.run_id != resulting.run_id || step.sequence != resulting.completed_steps {
            return Err(StoreError::InvalidRequest);
        }
        if observed_latest.is_some_and(|latest| {
            latest.orchestration_id != resulting.orchestration_id
                || latest.revision_id != resulting.revision_id
                || (latest.run_id != resulting.run_id
                    && latest.status != PortfolioRunStatus::Completed)
        }) {
            return Err(StoreError::InvalidRequest);
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StoreError::Sqlite)?;
        let latest = transaction
            .query_row(
                "SELECT run_id, orchestration_id, revision_id, current_node_id,
                        status, completed_steps, next_tick_at_epoch_seconds
                 FROM board_portfolio_runs
                 WHERE orchestration_id = ?1 AND revision_id = ?2
                 ORDER BY rowid DESC
                 LIMIT 1",
                params![resulting.orchestration_id, resulting.revision_id],
                portfolio_run_from_row,
            )
            .optional()
            .map_err(StoreError::Sqlite)?
            .transpose()?;
        if latest.as_ref() != observed_latest {
            return Err(StoreError::ConcurrentChange);
        }
        let stored = transaction
            .query_row(
                "SELECT run_id, orchestration_id, revision_id, current_node_id,
                        status, completed_steps, next_tick_at_epoch_seconds
                 FROM board_portfolio_runs WHERE run_id = ?1",
                [&resulting.run_id],
                portfolio_run_from_row,
            )
            .optional()
            .map_err(StoreError::Sqlite)?
            .transpose()?;
        let expected_same_run = observed_latest
            .filter(|latest| latest.run_id == resulting.run_id)
            .cloned();
        if stored != expected_same_run {
            return Err(StoreError::ConcurrentChange);
        }
        transaction
            .execute(
                "INSERT INTO board_portfolio_runs
                   (run_id, orchestration_id, revision_id, current_node_id, status,
                    completed_steps, next_tick_at_epoch_seconds)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(run_id) DO UPDATE SET
                   current_node_id = excluded.current_node_id,
                   status = excluded.status,
                   completed_steps = excluded.completed_steps,
                   next_tick_at_epoch_seconds = excluded.next_tick_at_epoch_seconds,
                   updated_at = CURRENT_TIMESTAMP",
                params![
                    resulting.run_id,
                    resulting.orchestration_id,
                    resulting.revision_id,
                    resulting.current_node_id,
                    resulting.status.as_str(),
                    resulting.completed_steps,
                    resulting.next_tick_at_epoch_seconds,
                ],
            )
            .map_err(StoreError::Sqlite)?;
        transaction
            .execute(
                "INSERT INTO board_portfolio_run_steps
                   (run_id, sequence, node_id, signal, destination_node_id,
                    selected_project_id, recorded_at_epoch_seconds)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    step.run_id,
                    step.sequence,
                    step.node_id,
                    step.signal.map(PortfolioSignal::as_str),
                    step.destination_node_id,
                    step.selected_project_id,
                    step.recorded_at_epoch_seconds,
                ],
            )
            .map_err(StoreError::Sqlite)?;
        transaction.commit().map_err(StoreError::Sqlite)
    }

    /// Atomically select one persisted Graph revision and entry for a Board project.
    ///
    /// # Errors
    ///
    /// Returns a bounded project, graph, validation, concurrent-change, or storage error.
    pub fn save_project_graph_binding(
        &mut self,
        request: &ProjectGraphBindingSaveRequest,
    ) -> Result<ProjectGraphBindingSaveReceipt, StoreError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StoreError::Sqlite)?;
        let project_exists: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM board_projects WHERE id = ?1)",
                [&request.target.project_id],
                |row| row.get(0),
            )
            .map_err(StoreError::Sqlite)?;
        if !project_exists {
            return Err(StoreError::ProjectNotFound);
        }
        let definition: Option<String> = transaction
            .query_row(
                "SELECT definition_json
                 FROM board_control_graph_revisions
                 WHERE graph_id = ?1 AND revision_id = ?2",
                params![request.target.graph_id, request.target.revision_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(StoreError::Sqlite)?;
        let Some(definition) = definition else {
            return Err(StoreError::GraphRevisionNotFound);
        };
        let graph: ControlGraphRevision =
            serde_json::from_str(&definition).map_err(StoreError::Json)?;
        graph
            .validate()
            .map_err(|_| StoreError::CorruptState("invalid Control graph revision"))?;
        request
            .target
            .validate_against(&graph)
            .map_err(|_| StoreError::InvalidRequest)?;

        let previous = transaction
            .query_row(
                "SELECT project_id, graph_id, revision_id, entry_id
                 FROM board_project_graph_bindings
                 WHERE project_id = ?1",
                [&request.target.project_id],
                |row| {
                    Ok(ProjectGraphBinding {
                        project_id: row.get(0)?,
                        graph_id: row.get(1)?,
                        revision_id: row.get(2)?,
                        entry_id: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(StoreError::Sqlite)?;
        if previous != request.expected {
            return Err(StoreError::ConcurrentChange);
        }
        let changed = previous.as_ref() != Some(&request.target);
        if changed {
            transaction
                .execute(
                    "INSERT INTO board_project_graph_bindings
                       (project_id, graph_id, revision_id, entry_id)
                     VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(project_id) DO UPDATE SET
                       graph_id = excluded.graph_id,
                       revision_id = excluded.revision_id,
                       entry_id = excluded.entry_id",
                    params![
                        request.target.project_id,
                        request.target.graph_id,
                        request.target.revision_id,
                        request.target.entry_id
                    ],
                )
                .map_err(StoreError::Sqlite)?;
        }
        transaction.commit().map_err(StoreError::Sqlite)?;
        Ok(ProjectGraphBindingSaveReceipt {
            previous,
            resulting: request.target.clone(),
            changed,
        })
    }

    /// Load the selected Graph revision and entry for every configured project.
    ///
    /// # Errors
    ///
    /// Returns a storage error.
    pub fn load_project_graph_bindings(&self) -> Result<Vec<ProjectGraphBinding>, StoreError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT project_id, graph_id, revision_id, entry_id
                 FROM board_project_graph_bindings
                 ORDER BY project_id",
            )
            .map_err(StoreError::Sqlite)?;
        let rows = statement
            .query_map([], |row| {
                Ok(ProjectGraphBinding {
                    project_id: row.get(0)?,
                    graph_id: row.get(1)?,
                    revision_id: row.get(2)?,
                    entry_id: row.get(3)?,
                })
            })
            .map_err(StoreError::Sqlite)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::Sqlite)
    }

    /// Record one pending candidate revision without publishing it to the Graph catalog.
    ///
    /// # Errors
    ///
    /// Returns a bounded validation, relationship, duplicate, concurrent-change,
    /// serialization, or storage error.
    pub fn save_graph_rewrite_proposal(
        &mut self,
        proposal: &GraphRewriteProposal,
    ) -> Result<GraphRewriteProposal, StoreError> {
        validate_graph_rewrite_proposal(proposal)?;
        if proposal.status != GraphRewriteProposalStatus::Pending {
            return Err(StoreError::InvalidRequest);
        }
        let definition = serde_json::to_string(proposal).map_err(StoreError::Json)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StoreError::Sqlite)?;
        let current = transaction
            .query_row(
                "SELECT project_id, graph_id, revision_id, entry_id
                 FROM board_project_graph_bindings
                 WHERE project_id = ?1",
                [&proposal.project_id],
                |row| {
                    Ok(ProjectGraphBinding {
                        project_id: row.get(0)?,
                        graph_id: row.get(1)?,
                        revision_id: row.get(2)?,
                        entry_id: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(StoreError::Sqlite)?
            .ok_or(StoreError::ProjectGraphBindingNotFound)?;
        if current != proposal.source_binding {
            return Err(StoreError::ConcurrentChange);
        }
        validate_graph_rewrite_relationships(&transaction, proposal)?;
        let candidate_exists: bool = transaction
            .query_row(
                "SELECT EXISTS(
                   SELECT 1 FROM board_control_graph_revisions
                   WHERE graph_id = ?1 AND revision_id = ?2
                 )",
                params![
                    proposal.candidate_graph.graph_id,
                    proposal.candidate_graph.revision_id
                ],
                |row| row.get(0),
            )
            .map_err(StoreError::Sqlite)?;
        if candidate_exists {
            return Err(StoreError::GraphRevisionAlreadyExists);
        }
        let inserted = transaction
            .execute(
                "INSERT OR IGNORE INTO board_graph_rewrite_proposals
                   (proposal_id, project_id, status, definition_json)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    proposal.proposal_id,
                    proposal.project_id,
                    proposal.status.as_str(),
                    definition
                ],
            )
            .map_err(StoreError::Sqlite)?;
        if inserted != 1 {
            return Err(StoreError::GraphRewriteProposalAlreadyExists);
        }
        transaction.commit().map_err(StoreError::Sqlite)?;
        Ok(proposal.clone())
    }

    /// Load every Graph rewrite proposal for one project in creation order.
    ///
    /// # Errors
    ///
    /// Returns a bounded validation, storage, serialization, or corrupt-state error.
    pub fn load_project_graph_rewrite_proposals(
        &self,
        project_id: &str,
    ) -> Result<Vec<GraphRewriteProposal>, StoreError> {
        validate_stable_identifier(project_id)?;
        let mut statement = self
            .connection
            .prepare(
                "SELECT status, definition_json
                 FROM board_graph_rewrite_proposals
                 WHERE project_id = ?1
                 ORDER BY created_at, proposal_id",
            )
            .map_err(StoreError::Sqlite)?;
        let rows = statement
            .query_map([project_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(StoreError::Sqlite)?;
        let mut proposals = Vec::new();
        for row in rows {
            let (status, definition) = row.map_err(StoreError::Sqlite)?;
            let proposal: GraphRewriteProposal =
                serde_json::from_str(&definition).map_err(StoreError::Json)?;
            validate_graph_rewrite_proposal(&proposal)
                .map_err(|_| StoreError::CorruptState("invalid Graph rewrite proposal"))?;
            if proposal.project_id != project_id || proposal.status.as_str() != status {
                return Err(StoreError::CorruptState(
                    "Graph rewrite proposal columns do not match its definition",
                ));
            }
            proposals.push(proposal);
        }
        Ok(proposals)
    }

    /// Atomically approve or reject one pending Graph rewrite proposal.
    ///
    /// Approval publishes the candidate revision and selects it for future work.
    /// Existing Work item graph positions remain pinned to their recorded revision.
    /// Rejection publishes nothing and preserves the current Project binding.
    ///
    /// # Errors
    ///
    /// Returns a bounded validation, lookup, already-decided, concurrent-change,
    /// conflicting-revision, serialization, corrupt-state, or storage error.
    pub fn decide_graph_rewrite_proposal(
        &mut self,
        request: &GraphRewriteDecisionRequest,
    ) -> Result<GraphRewriteDecisionReceipt, StoreError> {
        validate_stable_identifier(&request.proposal_id)?;
        validate_stable_identifier(&request.project_id)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StoreError::Sqlite)?;
        let stored: Option<(String, String)> = transaction
            .query_row(
                "SELECT status, definition_json
                 FROM board_graph_rewrite_proposals
                 WHERE proposal_id = ?1 AND project_id = ?2",
                params![request.proposal_id, request.project_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(StoreError::Sqlite)?;
        let Some((stored_status, definition)) = stored else {
            return Err(StoreError::GraphRewriteProposalNotFound);
        };
        let mut proposal: GraphRewriteProposal =
            serde_json::from_str(&definition).map_err(StoreError::Json)?;
        validate_graph_rewrite_proposal(&proposal)
            .map_err(|_| StoreError::CorruptState("invalid Graph rewrite proposal"))?;
        if proposal.project_id != request.project_id
            || proposal.proposal_id != request.proposal_id
            || proposal.status.as_str() != stored_status
        {
            return Err(StoreError::CorruptState(
                "Graph rewrite proposal columns do not match its definition",
            ));
        }
        if proposal.status != GraphRewriteProposalStatus::Pending {
            return Err(StoreError::GraphRewriteProposalAlreadyDecided);
        }
        validate_graph_rewrite_relationships(&transaction, &proposal).map_err(
            |error| match error {
                StoreError::InvalidRequest | StoreError::AgentProfileNotFound => {
                    StoreError::CorruptState("Graph rewrite proposal is not a bounded rewrite")
                }
                error => error,
            },
        )?;

        let previous_binding = transaction
            .query_row(
                "SELECT project_id, graph_id, revision_id, entry_id
                 FROM board_project_graph_bindings
                 WHERE project_id = ?1",
                [&request.project_id],
                |row| {
                    Ok(ProjectGraphBinding {
                        project_id: row.get(0)?,
                        graph_id: row.get(1)?,
                        revision_id: row.get(2)?,
                        entry_id: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(StoreError::Sqlite)?
            .ok_or(StoreError::ProjectGraphBindingNotFound)?;
        let (resulting_binding, published) = match request.decision {
            GraphRewriteDecision::Approve => {
                proposal.status = GraphRewriteProposalStatus::Approved;
                (
                    publish_graph_rewrite_candidate(&transaction, &proposal, &previous_binding)?,
                    true,
                )
            }
            GraphRewriteDecision::Reject => {
                proposal.status = GraphRewriteProposalStatus::Rejected;
                (previous_binding.clone(), false)
            }
        };
        let decided_definition = serde_json::to_string(&proposal).map_err(StoreError::Json)?;
        let changed = transaction
            .execute(
                "UPDATE board_graph_rewrite_proposals
                 SET status = ?1, definition_json = ?2, decided_at = CURRENT_TIMESTAMP
                 WHERE proposal_id = ?3 AND project_id = ?4 AND status = 'pending'",
                params![
                    proposal.status.as_str(),
                    decided_definition,
                    request.proposal_id,
                    request.project_id
                ],
            )
            .map_err(StoreError::Sqlite)?;
        if changed != 1 {
            return Err(StoreError::ConcurrentChange);
        }
        transaction.commit().map_err(StoreError::Sqlite)?;
        Ok(GraphRewriteDecisionReceipt {
            proposal,
            previous_binding,
            resulting_binding,
            published,
        })
    }

    /// Evaluate and atomically record one Work item's next declared Control route.
    ///
    /// The first decision pins the Work item to its project's current Graph revision
    /// and entry. Later project binding changes do not affect that position.
    ///
    /// # Errors
    ///
    /// Returns a bounded validation, relationship, route, concurrent-change,
    /// duplicate-identity, corrupt-state, serialization, or storage error.
    pub fn record_route_decision(
        &mut self,
        request: &RouteDecisionRequest,
    ) -> Result<RouteDecisionReceipt, StoreError> {
        validate_route_decision_request(request)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StoreError::Sqlite)?;

        let existing = transaction
            .query_row(
                &route_decision_select_sql("WHERE decision_id = ?1"),
                [&request.decision_id],
                raw_route_decision,
            )
            .optional()
            .map_err(StoreError::Sqlite)?
            .map(decode_route_decision)
            .transpose()?;
        if let Some(decision) = existing {
            if !route_decision_matches_request(&decision, request) {
                return Err(StoreError::RouteDecisionAlreadyExists);
            }
            let resulting_position = position_after(&decision);
            transaction.commit().map_err(StoreError::Sqlite)?;
            return Ok(RouteDecisionReceipt {
                decision,
                resulting_position,
                recorded: false,
            });
        }

        let work_item: Option<(String, String)> = transaction
            .query_row(
                "SELECT project_id, state FROM board_work_items WHERE id = ?1",
                [&request.work_item_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(StoreError::Sqlite)?;
        let Some((stored_project_id, stored_state)) = work_item else {
            return Err(StoreError::WorkItemNotFound);
        };
        if stored_project_id != request.project_id {
            return Err(StoreError::WorkItemNotFound);
        }
        let work_item_state = WorkItemState::try_from(stored_state.as_str())
            .map_err(|_| StoreError::CorruptState("unknown Work item state"))?;
        if !matches!(
            work_item_state,
            WorkItemState::Todo | WorkItemState::InProgress | WorkItemState::InReview
        ) {
            return Err(StoreError::InvalidRequest);
        }

        let existing_position = transaction
            .query_row(
                &graph_position_select_sql("WHERE work_item_id = ?1"),
                [&request.work_item_id],
                graph_position_from_row,
            )
            .optional()
            .map_err(StoreError::Sqlite)?;
        let position = match existing_position {
            Some(position) => position,
            None => {
                initial_graph_position(&transaction, &request.project_id, &request.work_item_id)?
            }
        };
        if position.current_node_id != request.expected_current_node_id {
            return Err(StoreError::ConcurrentChange);
        }
        let graph =
            load_control_graph_revision(&transaction, &position.graph_id, &position.revision_id)?;
        let selected = graph
            .select_route(
                &position.current_node_id,
                request.signal,
                request.proposed_route_id.as_deref(),
            )
            .map_err(|_| StoreError::RouteNotSelected)?;
        let decision = RouteDecision {
            decision_id: request.decision_id.clone(),
            project_id: request.project_id.clone(),
            work_item_id: request.work_item_id.clone(),
            graph_id: position.graph_id.clone(),
            revision_id: position.revision_id.clone(),
            entry_id: position.entry_id.clone(),
            source_node_id: selected.source_node_id,
            signal: selected.signal,
            proposed_route_id: request.proposed_route_id.clone(),
            route_id: selected.route_id,
            next_node_id: selected.next_node_id,
            evidence_refs: request.evidence_refs.clone(),
        };
        let resulting_position = position_after(&decision);
        persist_route_decision(&transaction, &decision, &resulting_position)?;
        transaction.commit().map_err(StoreError::Sqlite)?;
        Ok(RouteDecisionReceipt {
            decision,
            resulting_position,
            recorded: true,
        })
    }

    /// Load immutable Route decisions for one Work item in recorded order.
    ///
    /// # Errors
    ///
    /// Returns a bounded validation, storage, serialization, or corrupt-state error.
    pub fn load_route_decisions(
        &self,
        work_item_id: &str,
    ) -> Result<Vec<RouteDecision>, StoreError> {
        validate_work_item_reference(work_item_id)?;
        let sql = route_decision_select_sql("WHERE work_item_id = ?1 ORDER BY rowid");
        let mut statement = self.connection.prepare(&sql).map_err(StoreError::Sqlite)?;
        let rows = statement
            .query_map([work_item_id], raw_route_decision)
            .map_err(StoreError::Sqlite)?;
        let mut decisions = Vec::new();
        for row in rows {
            decisions.push(decode_route_decision(row.map_err(StoreError::Sqlite)?)?);
        }
        Ok(decisions)
    }

    /// Load immutable Route decisions for one Board project in recorded order.
    ///
    /// # Errors
    ///
    /// Returns a bounded validation, storage, serialization, or corrupt-state error.
    pub fn load_project_route_decisions(
        &self,
        project_id: &str,
    ) -> Result<Vec<RouteDecision>, StoreError> {
        validate_stable_identifier(project_id)?;
        let sql = route_decision_select_sql("WHERE project_id = ?1 ORDER BY rowid");
        let mut statement = self.connection.prepare(&sql).map_err(StoreError::Sqlite)?;
        let rows = statement
            .query_map([project_id], raw_route_decision)
            .map_err(StoreError::Sqlite)?;
        let mut decisions = Vec::new();
        for row in rows {
            decisions.push(decode_route_decision(row.map_err(StoreError::Sqlite)?)?);
        }
        Ok(decisions)
    }

    /// Load every Work item's current pinned Graph position.
    ///
    /// # Errors
    ///
    /// Returns a storage error.
    pub fn load_work_item_graph_positions(&self) -> Result<Vec<WorkItemGraphPosition>, StoreError> {
        let sql = graph_position_select_sql("ORDER BY project_id, work_item_id");
        let mut statement = self.connection.prepare(&sql).map_err(StoreError::Sqlite)?;
        let rows = statement
            .query_map([], graph_position_from_row)
            .map_err(StoreError::Sqlite)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::Sqlite)
    }

    /// Pin a Work item to its project's selected Graph entry and resolve the
    /// current Agent Loop into a concrete Runner target.
    ///
    /// Repeating this operation preserves the existing pinned revision and node.
    /// It does not create a Run or grant a Core capability.
    ///
    /// # Errors
    ///
    /// Returns a bounded validation, relationship, missing-binding,
    /// non-Agent-node, corrupt-state, serialization, or storage error.
    pub fn prepare_agent_loop_execution_target(
        &mut self,
        project_id: &str,
        work_item_id: &str,
    ) -> Result<AgentLoopExecutionTarget, StoreError> {
        validate_stable_identifier(project_id)?;
        validate_work_item_reference(work_item_id)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StoreError::Sqlite)?;
        let stored_work_item: Option<(String, String)> = transaction
            .query_row(
                "SELECT project_id, state FROM board_work_items WHERE id = ?1",
                [work_item_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(StoreError::Sqlite)?;
        let Some((stored_project_id, stored_state)) = stored_work_item else {
            return Err(StoreError::WorkItemNotFound);
        };
        if stored_project_id != project_id {
            return Err(StoreError::WorkItemNotFound);
        }
        let work_item_state = WorkItemState::try_from(stored_state.as_str())
            .map_err(|_| StoreError::CorruptState("unknown Work item state"))?;
        if !matches!(
            work_item_state,
            WorkItemState::Todo | WorkItemState::InProgress | WorkItemState::InReview
        ) {
            return Err(StoreError::InvalidRequest);
        }
        let existing = transaction
            .query_row(
                &graph_position_select_sql("WHERE work_item_id = ?1"),
                [work_item_id],
                graph_position_from_row,
            )
            .optional()
            .map_err(StoreError::Sqlite)?;
        let position = if let Some(position) = existing {
            if position.project_id != project_id {
                return Err(StoreError::CorruptState(
                    "Work item Graph position project differs",
                ));
            }
            position
        } else {
            let position = initial_graph_position(&transaction, project_id, work_item_id)?;
            transaction
                .execute(
                    "INSERT INTO board_work_item_graph_positions (
                       work_item_id, project_id, graph_id, revision_id, entry_id,
                       current_node_id
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        position.work_item_id,
                        position.project_id,
                        position.graph_id,
                        position.revision_id,
                        position.entry_id,
                        position.current_node_id
                    ],
                )
                .map_err(StoreError::Sqlite)?;
            position
        };
        let target = resolve_agent_loop_target(&transaction, &position)?;
        transaction.commit().map_err(StoreError::Sqlite)?;
        Ok(target)
    }

    /// Resolve one Work item's current Agent Loop node into a concrete Runner target.
    ///
    /// This returns Board scheduling identity only; Runner must still repeat behavior,
    /// workspace, approval, and Core capability preflight.
    ///
    /// # Errors
    ///
    /// Returns a bounded validation, missing-position, non-Agent-node, corrupt-state,
    /// serialization, or storage error.
    pub fn load_agent_loop_execution_target(
        &self,
        work_item_id: &str,
    ) -> Result<AgentLoopExecutionTarget, StoreError> {
        validate_work_item_reference(work_item_id)?;
        let position = self
            .connection
            .query_row(
                &graph_position_select_sql("WHERE work_item_id = ?1"),
                [work_item_id],
                graph_position_from_row,
            )
            .optional()
            .map_err(StoreError::Sqlite)?
            .ok_or(StoreError::WorkItemGraphPositionNotFound)?;
        resolve_agent_loop_target(&self.connection, &position)
    }

    /// Load the bounded Agent profile catalog used by Board scheduling.
    ///
    /// # Errors
    ///
    /// Returns a storage or corrupt-state error.
    pub fn load_agent_profiles(&self) -> Result<Vec<AgentProfileSummary>, StoreError> {
        let mut statement = self
            .connection
            .prepare("SELECT id, role, instruction_ref FROM board_agent_profiles ORDER BY id")
            .map_err(StoreError::Sqlite)?;
        let rows = statement
            .query_map([], |row| {
                Ok(AgentProfileSummary {
                    id: row.get(0)?,
                    role: row.get(1)?,
                    capabilities: Vec::new(),
                    instruction_ref: row.get(2)?,
                    skill_refs: Vec::new(),
                })
            })
            .map_err(StoreError::Sqlite)?;
        let mut profiles = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::Sqlite)?;
        drop(statement);

        let positions: HashMap<String, usize> = profiles
            .iter()
            .enumerate()
            .map(|(index, profile)| (profile.id.clone(), index))
            .collect();
        let mut capability_statement = self
            .connection
            .prepare(
                "SELECT agent_profile_id, capability
                 FROM board_agent_profile_capabilities
                 ORDER BY agent_profile_id, capability",
            )
            .map_err(StoreError::Sqlite)?;
        let capabilities = capability_statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(StoreError::Sqlite)?;
        for capability in capabilities {
            let (agent_profile_id, capability) = capability.map_err(StoreError::Sqlite)?;
            let index = positions
                .get(&agent_profile_id)
                .copied()
                .ok_or(StoreError::CorruptState("capability owner is missing"))?;
            profiles[index].capabilities.push(capability);
        }
        drop(capability_statement);

        let mut skill_statement = self
            .connection
            .prepare(
                "SELECT agent_profile_id, skill_ref
                 FROM board_agent_profile_skills
                 ORDER BY agent_profile_id, skill_ref",
            )
            .map_err(StoreError::Sqlite)?;
        let skill_refs = skill_statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(StoreError::Sqlite)?;
        for skill_ref in skill_refs {
            let (agent_profile_id, skill_ref) = skill_ref.map_err(StoreError::Sqlite)?;
            let index = positions
                .get(&agent_profile_id)
                .copied()
                .ok_or(StoreError::CorruptState("Skill reference owner is missing"))?;
            profiles[index].skill_refs.push(skill_ref);
        }
        Ok(profiles)
    }

    /// Load the locally selected Execution workspace for every Board project.
    ///
    /// # Errors
    ///
    /// Returns a storage or corrupt-state error.
    pub fn load_execution_workspaces(
        &self,
    ) -> Result<Vec<ExecutionWorkspaceConnection>, StoreError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT project_id, kind, location
                 FROM board_execution_workspaces
                 ORDER BY project_id",
            )
            .map_err(StoreError::Sqlite)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })
            .map_err(StoreError::Sqlite)?;
        rows.map(|row| {
            let (project_id, kind, location) = row.map_err(StoreError::Sqlite)?;
            let kind = ExecutionWorkspaceKind::try_from(kind.as_str())
                .map_err(|_| StoreError::CorruptState("unknown Execution workspace kind"))?;
            canonical_execution_workspace(&ExecutionWorkspaceConnection {
                project_id,
                kind,
                location,
            })
        })
        .collect()
    }

    /// Atomically replace one Board project's locally selected Execution workspace.
    ///
    /// # Errors
    ///
    /// Returns a bounded validation, lookup, concurrent-change, storage, or
    /// corrupt-state error. Saving the stored connection is a successful no-op.
    pub fn save_execution_workspace(
        &mut self,
        request: &ExecutionWorkspaceSaveRequest,
    ) -> Result<ExecutionWorkspaceSaveReceipt, StoreError> {
        let expected = request
            .expected
            .as_ref()
            .map(canonical_execution_workspace)
            .transpose()?;
        let target = canonical_execution_workspace(&request.target)?;
        if expected
            .as_ref()
            .is_some_and(|connection| connection.project_id != target.project_id)
        {
            return Err(StoreError::InvalidRequest);
        }

        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StoreError::Sqlite)?;
        let project_exists: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM board_projects WHERE id = ?1)",
                [&target.project_id],
                |row| row.get(0),
            )
            .map_err(StoreError::Sqlite)?;
        if !project_exists {
            return Err(StoreError::ProjectNotFound);
        }
        let stored = load_execution_workspace(&transaction, &target.project_id)?;
        match (&expected, &stored) {
            (None, Some(_)) | (Some(_), None) => return Err(StoreError::ConcurrentChange),
            (Some(observed), Some(current)) if observed != current => {
                return Err(StoreError::ConcurrentChange);
            }
            _ => {}
        }
        if stored.as_ref() == Some(&target) {
            return Ok(ExecutionWorkspaceSaveReceipt {
                previous: stored,
                resulting: target,
                changed: false,
            });
        }

        if stored.is_none() {
            transaction
                .execute(
                    "INSERT INTO board_execution_workspaces (project_id, kind, location)
                     VALUES (?1, ?2, ?3)",
                    params![target.project_id, target.kind.as_str(), target.location],
                )
                .map_err(StoreError::Sqlite)?;
        } else {
            let changed = transaction
                .execute(
                    "UPDATE board_execution_workspaces
                     SET kind = ?1, location = ?2
                     WHERE project_id = ?3",
                    params![target.kind.as_str(), target.location, target.project_id],
                )
                .map_err(StoreError::Sqlite)?;
            if changed != 1 {
                return Err(StoreError::ConcurrentChange);
            }
        }
        transaction.commit().map_err(StoreError::Sqlite)?;
        Ok(ExecutionWorkspaceSaveReceipt {
            previous: stored,
            resulting: target,
            changed: true,
        })
    }

    /// Create or atomically replace one Board-owned Agent profile.
    ///
    /// # Errors
    ///
    /// Returns a bounded validation, lookup, concurrent-change, storage, or
    /// corrupt-state error. Saving the stored profile is a successful no-op.
    pub fn save_agent_profile(
        &mut self,
        request: &AgentProfileSaveRequest,
    ) -> Result<AgentProfileSaveReceipt, StoreError> {
        let expected = request
            .expected
            .as_ref()
            .map(canonical_agent_profile)
            .transpose()?;
        let target = canonical_agent_profile(&request.target)?;
        if expected
            .as_ref()
            .is_some_and(|profile| profile.id != target.id)
        {
            return Err(StoreError::InvalidRequest);
        }

        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StoreError::Sqlite)?;
        let stored = load_agent_profile(&transaction, &target.id)?;
        match (&expected, &stored) {
            (None, Some(_)) => return Err(StoreError::AgentProfileAlreadyExists),
            (Some(_), None) => return Err(StoreError::AgentProfileNotFound),
            (Some(observed), Some(current)) if observed != current => {
                return Err(StoreError::ConcurrentChange);
            }
            _ => {}
        }
        if stored.as_ref() == Some(&target) {
            return Ok(AgentProfileSaveReceipt {
                previous: stored,
                resulting: target,
                changed: false,
            });
        }

        if stored.is_none() {
            transaction
                .execute(
                    "INSERT INTO board_agent_profiles (id, role, instruction_ref)
                     VALUES (?1, ?2, ?3)",
                    params![target.id, target.role, target.instruction_ref],
                )
                .map_err(StoreError::Sqlite)?;
        } else {
            let changed = transaction
                .execute(
                    "UPDATE board_agent_profiles
                     SET role = ?1, instruction_ref = ?2
                     WHERE id = ?3",
                    params![target.role, target.instruction_ref, target.id],
                )
                .map_err(StoreError::Sqlite)?;
            if changed != 1 {
                return Err(StoreError::ConcurrentChange);
            }
            transaction
                .execute(
                    "DELETE FROM board_agent_profile_capabilities
                     WHERE agent_profile_id = ?1",
                    [&target.id],
                )
                .map_err(StoreError::Sqlite)?;
            transaction
                .execute(
                    "DELETE FROM board_agent_profile_skills
                     WHERE agent_profile_id = ?1",
                    [&target.id],
                )
                .map_err(StoreError::Sqlite)?;
        }
        for capability in &target.capabilities {
            transaction
                .execute(
                    "INSERT INTO board_agent_profile_capabilities
                       (agent_profile_id, capability)
                     VALUES (?1, ?2)",
                    params![target.id, capability],
                )
                .map_err(StoreError::Sqlite)?;
        }
        for skill_ref in &target.skill_refs {
            transaction
                .execute(
                    "INSERT INTO board_agent_profile_skills
                       (agent_profile_id, skill_ref)
                     VALUES (?1, ?2)",
                    params![target.id, skill_ref],
                )
                .map_err(StoreError::Sqlite)?;
        }
        transaction.commit().map_err(StoreError::Sqlite)?;
        Ok(AgentProfileSaveReceipt {
            previous: stored,
            resulting: target,
            changed: true,
        })
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
                "SELECT id, project_id, title, priority, state, approval_requirement,
                        agent_profile_id
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
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                ))
            })
            .map_err(StoreError::Sqlite)?;
        let mut work_items = Vec::new();
        for row in rows {
            let (id, project_id, title, priority, state, approval_requirement, agent_profile_id) =
                row.map_err(StoreError::Sqlite)?;
            work_items.push(WorkItemSummary {
                id,
                project_id,
                title,
                priority: positive_u32(priority)?,
                state: parse_work_item_state(&state)?,
                approval_requirement: parse_approval_requirement(&approval_requirement)?,
                dependency_ids: Vec::new(),
                agent_profile_id,
                required_capabilities: Vec::new(),
            });
        }
        drop(statement);

        let positions: HashMap<String, usize> = work_items
            .iter()
            .enumerate()
            .map(|(index, work_item)| (work_item.id.clone(), index))
            .collect();
        hydrate_work_item_dependencies(&self.connection, &positions, &mut work_items)?;
        hydrate_work_item_requirements(&self.connection, &positions, &mut work_items)?;
        Ok(work_items)
    }

    /// Atomically replace one Work item's Board-owned Agent plan.
    ///
    /// # Errors
    ///
    /// Returns a bounded validation, lookup, concurrent-change, storage, or
    /// corrupt-state error. Saving the stored plan is a successful no-op.
    pub fn update_agent_plan(
        &mut self,
        request: &AgentPlanUpdateRequest,
    ) -> Result<AgentPlanUpdateReceipt, StoreError> {
        validate_id(&request.project_id)?;
        validate_id(&request.work_item_id)?;
        let expected = canonical_agent_plan(&request.expected)?;
        let target = canonical_agent_plan(&request.target)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StoreError::Sqlite)?;
        let stored = transaction
            .query_row(
                "SELECT w.id, w.agent_profile_id
                 FROM board_projects p
                 LEFT JOIN board_work_items w
                   ON w.project_id = p.id AND w.id = ?2
                 WHERE p.id = ?1",
                params![request.project_id, request.work_item_id],
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
        let (Some(_), stored_agent_profile_id) = stored else {
            return Err(StoreError::WorkItemNotFound);
        };
        let previous = AgentPlan {
            agent_profile_id: stored_agent_profile_id,
            required_capabilities: load_required_capabilities(&transaction, &request.work_item_id)?,
        };
        if previous != expected {
            return Err(StoreError::ConcurrentChange);
        }
        if let Some(agent_profile_id) = &target.agent_profile_id {
            let profile_exists = transaction
                .query_row(
                    "SELECT EXISTS(
                       SELECT 1 FROM board_agent_profiles WHERE id = ?1
                     )",
                    [agent_profile_id],
                    |row| row.get::<_, bool>(0),
                )
                .map_err(StoreError::Sqlite)?;
            if !profile_exists {
                return Err(StoreError::AgentProfileNotFound);
            }
        }
        if previous == target {
            return Ok(AgentPlanUpdateReceipt {
                work_item_id: request.work_item_id.clone(),
                previous: previous.clone(),
                resulting: previous,
                changed: false,
            });
        }

        let changed = transaction
            .execute(
                "UPDATE board_work_items
                 SET agent_profile_id = ?1
                 WHERE project_id = ?2 AND id = ?3",
                params![
                    target.agent_profile_id,
                    request.project_id,
                    request.work_item_id
                ],
            )
            .map_err(StoreError::Sqlite)?;
        if changed != 1 {
            return Err(StoreError::ConcurrentChange);
        }
        transaction
            .execute(
                "DELETE FROM board_work_item_required_capabilities
                 WHERE work_item_id = ?1",
                [&request.work_item_id],
            )
            .map_err(StoreError::Sqlite)?;
        for capability in &target.required_capabilities {
            transaction
                .execute(
                    "INSERT INTO board_work_item_required_capabilities
                       (work_item_id, capability)
                     VALUES (?1, ?2)",
                    params![request.work_item_id, capability],
                )
                .map_err(StoreError::Sqlite)?;
        }
        transaction.commit().map_err(StoreError::Sqlite)?;
        Ok(AgentPlanUpdateReceipt {
            work_item_id: request.work_item_id.clone(),
            previous,
            resulting: target,
            changed: true,
        })
    }

    /// Create one direct Board-owned Work item with safe scheduling defaults.
    ///
    /// # Errors
    ///
    /// Returns a bounded validation, lookup, duplicate-item, storage, or corrupt-state error.
    pub fn create_work_item(
        &mut self,
        request: &WorkItemCreateRequest,
    ) -> Result<WorkItemCreateReceipt, StoreError> {
        validate_id(&request.project_id)?;
        validate_id(&request.work_item_id)?;
        validate_title(&request.title)?;
        if request.title.trim() != request.title || request.priority == 0 {
            return Err(StoreError::InvalidRequest);
        }

        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StoreError::Sqlite)?;
        let project_exists: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM board_projects WHERE id = ?1)",
                [&request.project_id],
                |row| row.get(0),
            )
            .map_err(StoreError::Sqlite)?;
        if !project_exists {
            return Err(StoreError::ProjectNotFound);
        }
        let work_item_exists: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM board_work_items WHERE id = ?1)",
                [&request.work_item_id],
                |row| row.get(0),
            )
            .map_err(StoreError::Sqlite)?;
        if work_item_exists {
            return Err(StoreError::WorkItemAlreadyExists);
        }
        transaction
            .execute(
                "INSERT INTO board_work_items (id, project_id, title, priority, state)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    request.work_item_id,
                    request.project_id,
                    request.title,
                    request.priority,
                    WorkItemState::Todo.as_str()
                ],
            )
            .map_err(StoreError::Sqlite)?;
        transaction.commit().map_err(StoreError::Sqlite)?;

        Ok(WorkItemCreateReceipt {
            work_item: WorkItemSummary {
                id: request.work_item_id.clone(),
                project_id: request.project_id.clone(),
                title: request.title.clone(),
                priority: request.priority,
                state: WorkItemState::Todo,
                approval_requirement: ApprovalRequirement::None,
                dependency_ids: Vec::new(),
                agent_profile_id: None,
                required_capabilities: Vec::new(),
            },
        })
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

fn migrate_work_item_columns(connection: &Connection) -> Result<(), StoreError> {
    if !work_item_column_exists(connection, "priority")? {
        connection
            .execute(
                "ALTER TABLE board_work_items
                 ADD COLUMN priority INTEGER NOT NULL DEFAULT 100 CHECK (priority > 0)",
                [],
            )
            .map_err(StoreError::Sqlite)?;
    }
    if !work_item_column_exists(connection, "approval_requirement")? {
        connection
            .execute(
                "ALTER TABLE board_work_items
                 ADD COLUMN approval_requirement TEXT NOT NULL DEFAULT 'none' CHECK (
                   approval_requirement IN ('none', 'explicit')
                 )",
                [],
            )
            .map_err(StoreError::Sqlite)?;
    }
    if !work_item_column_exists(connection, "agent_profile_id")? {
        connection
            .execute(
                "ALTER TABLE board_work_items
                 ADD COLUMN agent_profile_id TEXT REFERENCES board_agent_profiles(id)",
                [],
            )
            .map_err(StoreError::Sqlite)?;
    }
    Ok(())
}

fn migrate_agent_profile_columns(connection: &Connection) -> Result<(), StoreError> {
    if !table_column_exists(connection, "board_agent_profiles", "instruction_ref")? {
        connection
            .execute(
                "ALTER TABLE board_agent_profiles ADD COLUMN instruction_ref TEXT",
                [],
            )
            .map_err(StoreError::Sqlite)?;
    }
    Ok(())
}

fn create_execution_workspace_storage(connection: &Connection) -> Result<(), StoreError> {
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS board_execution_workspaces (
               project_id TEXT PRIMARY KEY,
               kind TEXT NOT NULL CHECK (kind IN ('bundled_sample', 'local_directory')),
               location TEXT,
               FOREIGN KEY (project_id) REFERENCES board_projects(id) ON DELETE CASCADE
             );",
        )
        .map_err(StoreError::Sqlite)
}

fn create_auxiliary_storage(connection: &Connection) -> Result<(), StoreError> {
    create_execution_workspace_storage(connection)?;
    create_control_graph_storage(connection)?;
    create_portfolio_orchestration_storage(connection)?;
    create_orchestration_blueprint_storage(connection)
}

fn create_orchestration_blueprint_storage(connection: &Connection) -> Result<(), StoreError> {
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS board_orchestration_blueprint_revisions (
               blueprint_id TEXT NOT NULL,
               revision_id TEXT NOT NULL,
               definition_json TEXT NOT NULL,
               created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
               PRIMARY KEY (blueprint_id, revision_id)
             );
             CREATE TABLE IF NOT EXISTS board_blueprint_applications (
               application_id TEXT PRIMARY KEY,
               blueprint_id TEXT NOT NULL,
               revision_id TEXT NOT NULL,
               entry_node_id TEXT NOT NULL,
               project_id TEXT NOT NULL,
               work_item_id TEXT NOT NULL,
               created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
               FOREIGN KEY (blueprint_id, revision_id)
                 REFERENCES board_orchestration_blueprint_revisions(blueprint_id, revision_id),
               FOREIGN KEY (project_id) REFERENCES board_projects(id) ON DELETE CASCADE,
               FOREIGN KEY (work_item_id) REFERENCES board_work_items(id) ON DELETE CASCADE
             );
             CREATE TABLE IF NOT EXISTS board_blueprint_runtime_bindings (
               application_id TEXT PRIMARY KEY,
               project_id TEXT NOT NULL,
               work_item_id TEXT NOT NULL,
               agent_profile_id TEXT NOT NULL,
               execution_workspace_kind TEXT NOT NULL CHECK (
                 execution_workspace_kind IN ('bundled_sample', 'local_directory')
               ),
               execution_workspace_location TEXT,
               approach_notes_json TEXT NOT NULL,
               created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
               FOREIGN KEY (application_id)
                 REFERENCES board_blueprint_applications(application_id) ON DELETE CASCADE,
               FOREIGN KEY (project_id) REFERENCES board_projects(id) ON DELETE CASCADE,
               FOREIGN KEY (work_item_id) REFERENCES board_work_items(id) ON DELETE CASCADE,
               FOREIGN KEY (agent_profile_id) REFERENCES board_agent_profiles(id)
             );",
        )
        .map_err(StoreError::Sqlite)
}

fn create_portfolio_orchestration_storage(connection: &Connection) -> Result<(), StoreError> {
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS board_portfolio_orchestration_revisions (
               orchestration_id TEXT NOT NULL,
               revision_id TEXT NOT NULL,
               definition_json TEXT NOT NULL,
               created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
               PRIMARY KEY (orchestration_id, revision_id)
             );
             CREATE TABLE IF NOT EXISTS board_portfolio_runs (
               run_id TEXT PRIMARY KEY,
               orchestration_id TEXT NOT NULL,
               revision_id TEXT NOT NULL,
               current_node_id TEXT NOT NULL,
               status TEXT NOT NULL CHECK (
                 status IN ('active', 'waiting_approval', 'paused', 'completed')
               ),
               completed_steps INTEGER NOT NULL CHECK (completed_steps >= 0),
               next_tick_at_epoch_seconds INTEGER,
               created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
               updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
               FOREIGN KEY (orchestration_id, revision_id)
                 REFERENCES board_portfolio_orchestration_revisions(orchestration_id, revision_id)
             );
             CREATE TABLE IF NOT EXISTS board_portfolio_schedule_controls (
               orchestration_id TEXT NOT NULL,
               revision_id TEXT NOT NULL,
               automatic_ticks_enabled INTEGER NOT NULL CHECK (
                 automatic_ticks_enabled IN (0, 1)
               ),
               updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
               PRIMARY KEY (orchestration_id, revision_id),
               FOREIGN KEY (orchestration_id, revision_id)
                 REFERENCES board_portfolio_orchestration_revisions(orchestration_id, revision_id)
             );
             CREATE TABLE IF NOT EXISTS board_portfolio_run_steps (
               run_id TEXT NOT NULL,
               sequence INTEGER NOT NULL CHECK (sequence > 0),
               node_id TEXT NOT NULL,
               signal TEXT CHECK (
                 signal IS NULL OR signal IN (
                   'completed', 'no_candidate', 'needs_attention', 'failed',
                   'approved', 'rejected', 'manual'
                 )
               ),
               destination_node_id TEXT,
               selected_project_id TEXT,
               recorded_at_epoch_seconds INTEGER NOT NULL,
               PRIMARY KEY (run_id, sequence),
               FOREIGN KEY (run_id) REFERENCES board_portfolio_runs(run_id) ON DELETE CASCADE,
               FOREIGN KEY (selected_project_id) REFERENCES board_projects(id) ON DELETE SET NULL
             );",
        )
        .map_err(StoreError::Sqlite)
}

fn portfolio_run_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<Result<PortfolioRun, StoreError>> {
    let status: String = row.get(4)?;
    let completed_steps: i64 = row.get(5)?;
    let Ok(status) = PortfolioRunStatus::try_from(status.as_str()) else {
        return Ok(Err(StoreError::CorruptState(
            "unknown Portfolio Run status",
        )));
    };
    let Ok(completed_steps) = u32::try_from(completed_steps) else {
        return Ok(Err(StoreError::CorruptState(
            "invalid Portfolio Run step count",
        )));
    };
    Ok(Ok(PortfolioRun {
        run_id: row.get(0)?,
        orchestration_id: row.get(1)?,
        revision_id: row.get(2)?,
        current_node_id: row.get(3)?,
        status,
        completed_steps,
        next_tick_at_epoch_seconds: row.get(6)?,
    }))
}

fn portfolio_run_step_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<Result<PortfolioRunStep, StoreError>> {
    let sequence: i64 = row.get(1)?;
    let signal: Option<String> = row.get(3)?;
    let Ok(sequence) = u32::try_from(sequence) else {
        return Ok(Err(StoreError::CorruptState(
            "invalid Portfolio Run step sequence",
        )));
    };
    let signal = match signal {
        Some(signal) => match PortfolioSignal::try_from(signal.as_str()) {
            Ok(signal) => Some(signal),
            Err(_) => {
                return Ok(Err(StoreError::CorruptState(
                    "unknown Portfolio Run signal",
                )));
            }
        },
        None => None,
    };
    Ok(Ok(PortfolioRunStep {
        run_id: row.get(0)?,
        sequence,
        node_id: row.get(2)?,
        signal,
        destination_node_id: row.get(4)?,
        selected_project_id: row.get(5)?,
        recorded_at_epoch_seconds: row.get(6)?,
    }))
}

fn create_control_graph_storage(connection: &Connection) -> Result<(), StoreError> {
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS board_control_graph_revisions (
               graph_id TEXT NOT NULL,
               revision_id TEXT NOT NULL,
               definition_json TEXT NOT NULL,
               created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
               PRIMARY KEY (graph_id, revision_id)
             );
             CREATE TABLE IF NOT EXISTS board_project_graph_bindings (
               project_id TEXT PRIMARY KEY,
               graph_id TEXT NOT NULL,
               revision_id TEXT NOT NULL,
               entry_id TEXT NOT NULL,
               FOREIGN KEY (project_id) REFERENCES board_projects(id) ON DELETE CASCADE,
               FOREIGN KEY (graph_id, revision_id)
                 REFERENCES board_control_graph_revisions(graph_id, revision_id)
             );
             CREATE TABLE IF NOT EXISTS board_graph_canvas_layouts (
               graph_id TEXT PRIMARY KEY,
               layout_json TEXT NOT NULL,
               updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
             );
             CREATE TABLE IF NOT EXISTS board_work_item_graph_positions (
               work_item_id TEXT PRIMARY KEY,
               project_id TEXT NOT NULL,
               graph_id TEXT NOT NULL,
               revision_id TEXT NOT NULL,
               entry_id TEXT NOT NULL,
               current_node_id TEXT NOT NULL,
               FOREIGN KEY (work_item_id) REFERENCES board_work_items(id) ON DELETE CASCADE,
               FOREIGN KEY (project_id) REFERENCES board_projects(id) ON DELETE CASCADE,
               FOREIGN KEY (graph_id, revision_id)
                 REFERENCES board_control_graph_revisions(graph_id, revision_id)
             );
             CREATE TABLE IF NOT EXISTS board_route_decisions (
               decision_id TEXT PRIMARY KEY,
               project_id TEXT NOT NULL,
               work_item_id TEXT NOT NULL,
               graph_id TEXT NOT NULL,
               revision_id TEXT NOT NULL,
               entry_id TEXT NOT NULL,
               source_node_id TEXT NOT NULL,
               signal_json TEXT NOT NULL,
               proposed_route_id TEXT,
               route_id TEXT NOT NULL,
               next_node_id TEXT NOT NULL,
               evidence_refs_json TEXT NOT NULL,
               decided_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
               FOREIGN KEY (project_id) REFERENCES board_projects(id) ON DELETE CASCADE,
               FOREIGN KEY (work_item_id) REFERENCES board_work_items(id) ON DELETE CASCADE,
               FOREIGN KEY (graph_id, revision_id)
                 REFERENCES board_control_graph_revisions(graph_id, revision_id)
             );
             CREATE TABLE IF NOT EXISTS board_graph_rewrite_proposals (
               proposal_id TEXT PRIMARY KEY,
               project_id TEXT NOT NULL,
               status TEXT NOT NULL CHECK (status IN ('pending', 'approved', 'rejected')),
               definition_json TEXT NOT NULL,
               created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
               decided_at TEXT,
               FOREIGN KEY (project_id) REFERENCES board_projects(id) ON DELETE CASCADE
             );",
        )
        .map_err(StoreError::Sqlite)
}

fn set_schema_version(connection: &Connection) -> Result<(), StoreError> {
    connection
        .execute_batch("PRAGMA user_version = 15;")
        .map_err(StoreError::Sqlite)
}

fn insert_sample_projects(transaction: &rusqlite::Transaction<'_>) -> Result<(), StoreError> {
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
    Ok(())
}

fn builtin_control_graphs() -> [ControlGraphRevision; 3] {
    [direct_graph(), high_risk_graph(), reviewed_graph()]
}

fn builtin_orchestration_blueprints() -> [OrchestrationBlueprintRevision; 1] {
    [OrchestrationBlueprintRevision {
        blueprint_id: "evidence-first".to_owned(),
        revision_id: "v1".to_owned(),
        name: "Evidence-first delivery".to_owned(),
        scope: BlueprintScope::Project,
        entry_node_id: "approach".to_owned(),
        nodes: vec![
            BlueprintNode {
                id: "approach".to_owned(),
                kind: BlueprintNodeKind::Approach {
                    approach_id: "evidence-first".to_owned(),
                },
                inputs: vec![NoteSocketKind::Context, NoteSocketKind::WorkItem],
                outputs: vec![NoteSocketKind::Evidence],
            },
            BlueprintNode {
                id: "audit".to_owned(),
                kind: BlueprintNodeKind::Audit,
                inputs: vec![NoteSocketKind::Evidence],
                outputs: Vec::new(),
            },
            BlueprintNode {
                id: "finish".to_owned(),
                kind: BlueprintNodeKind::Terminal,
                inputs: Vec::new(),
                outputs: Vec::new(),
            },
        ],
        links: vec![
            BlueprintLink {
                id: "approach-audit-flow".to_owned(),
                source_node_id: "approach".to_owned(),
                destination_node_id: "audit".to_owned(),
                kind: BlueprintLinkKind::Flow {
                    signal: ControlSignal::Succeeded,
                },
            },
            BlueprintLink {
                id: "approach-audit-evidence".to_owned(),
                source_node_id: "approach".to_owned(),
                destination_node_id: "audit".to_owned(),
                kind: BlueprintLinkKind::Data {
                    socket: NoteSocketKind::Evidence,
                },
            },
            BlueprintLink {
                id: "audit-finish".to_owned(),
                source_node_id: "audit".to_owned(),
                destination_node_id: "finish".to_owned(),
                kind: BlueprintLinkKind::Flow {
                    signal: ControlSignal::Passed,
                },
            },
        ],
    }]
}

fn builtin_portfolio_orchestrations() -> [PortfolioOrchestrationRevision; 1] {
    [PortfolioOrchestrationRevision {
        orchestration_id: "managed-products".to_owned(),
        revision_id: "v2".to_owned(),
        name: "Managed products orchestration".to_owned(),
        schedule: PortfolioSchedule::Interval {
            every_minutes: 60,
            enabled: true,
        },
        entry_node_id: "select-project".to_owned(),
        nodes: vec![
            PortfolioNode {
                id: "select-project".to_owned(),
                kind: PortfolioNodeKind::ProjectSelector {
                    selector: PortfolioProjectSelector::AllManaged,
                },
            },
            PortfolioNode {
                id: "summary".to_owned(),
                kind: PortfolioNodeKind::PostAction {
                    action: PortfolioPostAction::RecordSummary,
                },
            },
            PortfolioNode {
                id: "finish".to_owned(),
                kind: PortfolioNodeKind::Terminal,
            },
        ],
        routes: vec![
            portfolio_route(
                "selection-completed",
                "select-project",
                "summary",
                PortfolioSignal::Completed,
            ),
            portfolio_route(
                "selection-empty",
                "select-project",
                "summary",
                PortfolioSignal::NoCandidate,
            ),
            portfolio_route(
                "selection-attention",
                "select-project",
                "summary",
                PortfolioSignal::NeedsAttention,
            ),
            portfolio_route(
                "summary-recorded",
                "summary",
                "finish",
                PortfolioSignal::Completed,
            ),
        ],
    }]
}

fn portfolio_route(
    id: &str,
    source: &str,
    destination: &str,
    signal: PortfolioSignal,
) -> PortfolioRoute {
    PortfolioRoute {
        id: id.to_owned(),
        source_node_id: source.to_owned(),
        destination_node_id: destination.to_owned(),
        signal,
    }
}

fn direct_graph() -> ControlGraphRevision {
    ControlGraphRevision {
        graph_id: "direct".to_owned(),
        revision_id: "v1".to_owned(),
        entries: vec![GraphEntry {
            id: "standard".to_owned(),
            node_id: "implement".to_owned(),
        }],
        nodes: vec![
            agent_node("implement", "implementer"),
            ControlNode {
                id: "verify".to_owned(),
                kind: ControlNodeKind::Audit,
            },
            terminal_node(),
        ],
        routes: vec![
            route(
                "implementation-complete",
                "implement",
                "verify",
                ControlSignal::Succeeded,
            ),
            route(
                "verification-passed",
                "verify",
                "finish",
                ControlSignal::Passed,
            ),
        ],
        anchors: vec![anchor(
            "verified-result",
            "Completion requires independently observed verification evidence",
        )],
    }
}

fn reviewed_graph() -> ControlGraphRevision {
    ControlGraphRevision {
        graph_id: "reviewed".to_owned(),
        revision_id: "v1".to_owned(),
        entries: vec![
            GraphEntry {
                id: "standard".to_owned(),
                node_id: "research".to_owned(),
            },
            GraphEntry {
                id: "implementation-only".to_owned(),
                node_id: "implement".to_owned(),
            },
        ],
        nodes: vec![
            agent_node("research", "researcher"),
            agent_node("implement", "implementer"),
            agent_node("review", "reviewer"),
            ControlNode {
                id: "verify".to_owned(),
                kind: ControlNodeKind::Audit,
            },
            terminal_node(),
        ],
        routes: vec![
            route(
                "research-complete",
                "research",
                "implement",
                ControlSignal::Succeeded,
            ),
            route(
                "implementation-complete",
                "implement",
                "review",
                ControlSignal::Succeeded,
            ),
            route("review-passed", "review", "verify", ControlSignal::Passed),
            route(
                "verification-passed",
                "verify",
                "finish",
                ControlSignal::Passed,
            ),
        ],
        anchors: vec![anchor(
            "independent-review",
            "Completion requires review and verification outside the implementation loop",
        )],
    }
}

fn high_risk_graph() -> ControlGraphRevision {
    ControlGraphRevision {
        graph_id: "high-risk".to_owned(),
        revision_id: "v1".to_owned(),
        entries: vec![GraphEntry {
            id: "standard".to_owned(),
            node_id: "plan".to_owned(),
        }],
        nodes: vec![
            agent_node("plan", "researcher"),
            ControlNode {
                id: "start-approval".to_owned(),
                kind: ControlNodeKind::Approval,
            },
            agent_node("implement", "implementer"),
            ControlNode {
                id: "audit".to_owned(),
                kind: ControlNodeKind::Audit,
            },
            ControlNode {
                id: "acceptance".to_owned(),
                kind: ControlNodeKind::Approval,
            },
            terminal_node(),
        ],
        routes: vec![
            route(
                "plan-ready",
                "plan",
                "start-approval",
                ControlSignal::NeedsApproval,
            ),
            route(
                "start-approved",
                "start-approval",
                "implement",
                ControlSignal::Approved,
            ),
            route(
                "implementation-complete",
                "implement",
                "audit",
                ControlSignal::Succeeded,
            ),
            route("audit-passed", "audit", "acceptance", ControlSignal::Passed),
            route("accepted", "acceptance", "finish", ControlSignal::Approved),
        ],
        anchors: vec![anchor(
            "human-authority",
            "Human approval is required before execution and final acceptance",
        )],
    }
}

fn agent_node(id: &str, agent_profile_id: &str) -> ControlNode {
    ControlNode {
        id: id.to_owned(),
        kind: ControlNodeKind::AgentLoop {
            agent_profile_id: agent_profile_id.to_owned(),
        },
    }
}

fn terminal_node() -> ControlNode {
    ControlNode {
        id: "finish".to_owned(),
        kind: ControlNodeKind::Terminal,
    }
}

fn route(id: &str, source: &str, destination: &str, signal: ControlSignal) -> ControlRoute {
    ControlRoute {
        id: id.to_owned(),
        source_node_id: source.to_owned(),
        destination_node_id: destination.to_owned(),
        signal,
    }
}

fn anchor(id: &str, description: &str) -> GraphAnchor {
    GraphAnchor {
        id: id.to_owned(),
        description: description.to_owned(),
    }
}

fn insert_sample_execution_workspaces(
    transaction: &rusqlite::Transaction<'_>,
) -> Result<(), StoreError> {
    for project_id in ["gareji-board", "gareji-core", "zettelkasten-plugin"] {
        transaction
            .execute(
                "INSERT INTO board_execution_workspaces (project_id, kind, location)
                 VALUES (?1, 'bundled_sample', NULL)",
                [project_id],
            )
            .map_err(StoreError::Sqlite)?;
    }
    Ok(())
}

fn insert_sample_agent_profiles(transaction: &rusqlite::Transaction<'_>) -> Result<(), StoreError> {
    let profiles = [
        (
            "implementer",
            "Implementer",
            "agents/implementer/AGENT.md",
            ["implementation", "testing"],
            ["implement-bounded-work-item", "write-project-handoff"],
        ),
        (
            "researcher",
            "Researcher",
            "agents/researcher/AGENT.md",
            ["evidence", "research"],
            ["summarize-project-context", "write-project-handoff"],
        ),
        (
            "reviewer",
            "Reviewer",
            "agents/reviewer/AGENT.md",
            ["review", "testing"],
            ["review-work-item", "write-project-handoff"],
        ),
        (
            "release-checker",
            "Release checker",
            "agents/release-checker/AGENT.md",
            ["release", "testing"],
            ["review-work-item", "write-project-handoff"],
        ),
    ];
    for (id, role, instruction_ref, capabilities, skill_refs) in profiles {
        transaction
            .execute(
                "INSERT INTO board_agent_profiles (id, role, instruction_ref)
                 VALUES (?1, ?2, ?3)",
                params![id, role, instruction_ref],
            )
            .map_err(StoreError::Sqlite)?;
        for capability in capabilities {
            transaction
                .execute(
                    "INSERT INTO board_agent_profile_capabilities
                       (agent_profile_id, capability)
                     VALUES (?1, ?2)",
                    params![id, capability],
                )
                .map_err(StoreError::Sqlite)?;
        }
        for skill_ref in skill_refs {
            transaction
                .execute(
                    "INSERT INTO board_agent_profile_skills
                       (agent_profile_id, skill_ref)
                     VALUES (?1, ?2)",
                    params![id, skill_ref],
                )
                .map_err(StoreError::Sqlite)?;
        }
    }
    Ok(())
}

fn insert_sample_work_items(transaction: &rusqlite::Transaction<'_>) -> Result<(), StoreError> {
    let work_items = [
        (
            "BOARD-1",
            "gareji-board",
            "Build portfolio screen",
            1_i64,
            ApprovalRequirement::None,
            "implementer",
            WorkItemState::InProgress,
        ),
        (
            "BOARD-2",
            "gareji-board",
            "Connect existing project",
            2_i64,
            ApprovalRequirement::Explicit,
            "implementer",
            WorkItemState::Todo,
        ),
        (
            "BOARD-3",
            "gareji-board",
            "Prepare cross-project handoff",
            3_i64,
            ApprovalRequirement::None,
            "researcher",
            WorkItemState::Todo,
        ),
        (
            "BOARD-4",
            "gareji-board",
            "Prepare release verification",
            4_i64,
            ApprovalRequirement::None,
            "researcher",
            WorkItemState::Todo,
        ),
        (
            "CORE-1",
            "gareji-core",
            "Stabilize Runner seam",
            1_i64,
            ApprovalRequirement::None,
            "reviewer",
            WorkItemState::InReview,
        ),
        (
            "CORE-2",
            "gareji-core",
            "Add active-work registry",
            1_i64,
            ApprovalRequirement::None,
            "implementer",
            WorkItemState::Todo,
        ),
        (
            "ZETTEL-1",
            "zettelkasten-plugin",
            "Resolve write destination",
            1_i64,
            ApprovalRequirement::None,
            "researcher",
            WorkItemState::Blocked,
        ),
    ];
    for work_item in work_items {
        transaction
            .execute(
                "INSERT INTO board_work_items
                   (id, project_id, title, priority, approval_requirement, agent_profile_id, state)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    work_item.0,
                    work_item.1,
                    work_item.2,
                    work_item.3,
                    work_item.4.as_str(),
                    work_item.5,
                    work_item.6.as_str()
                ],
            )
            .map_err(StoreError::Sqlite)?;
    }
    insert_sample_work_item_relationships(transaction)
}

fn insert_sample_work_item_relationships(
    transaction: &rusqlite::Transaction<'_>,
) -> Result<(), StoreError> {
    transaction
        .execute(
            "INSERT INTO board_work_item_dependencies
               (work_item_id, dependency_work_item_id)
             VALUES ('BOARD-3', 'CORE-1')",
            [],
        )
        .map_err(StoreError::Sqlite)?;
    let requirements = [
        ("BOARD-1", "implementation"),
        ("BOARD-2", "implementation"),
        ("BOARD-3", "research"),
        ("BOARD-4", "release"),
        ("CORE-1", "review"),
        ("CORE-2", "implementation"),
        ("ZETTEL-1", "research"),
    ];
    for (work_item_id, capability) in requirements {
        transaction
            .execute(
                "INSERT INTO board_work_item_required_capabilities
                   (work_item_id, capability)
                 VALUES (?1, ?2)",
                params![work_item_id, capability],
            )
            .map_err(StoreError::Sqlite)?;
    }
    Ok(())
}

fn work_item_column_exists(connection: &Connection, column: &str) -> Result<bool, StoreError> {
    table_column_exists(connection, "board_work_items", column)
}

fn table_column_exists(
    connection: &Connection,
    table: &str,
    column: &str,
) -> Result<bool, StoreError> {
    connection
        .query_row(
            "SELECT EXISTS(
               SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2
             )",
            params![table, column],
            |row| row.get(0),
        )
        .map_err(StoreError::Sqlite)
}

fn hydrate_work_item_dependencies(
    connection: &Connection,
    positions: &HashMap<String, usize>,
    work_items: &mut [WorkItemSummary],
) -> Result<(), StoreError> {
    let mut statement = connection
        .prepare(
            "SELECT work_item_id, dependency_work_item_id
             FROM board_work_item_dependencies
             ORDER BY work_item_id, dependency_work_item_id",
        )
        .map_err(StoreError::Sqlite)?;
    let dependencies = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(StoreError::Sqlite)?;
    for dependency in dependencies {
        let (work_item_id, dependency_id) = dependency.map_err(StoreError::Sqlite)?;
        let index = positions
            .get(&work_item_id)
            .copied()
            .ok_or(StoreError::CorruptState("dependency owner is missing"))?;
        work_items[index].dependency_ids.push(dependency_id);
    }
    Ok(())
}

fn hydrate_work_item_requirements(
    connection: &Connection,
    positions: &HashMap<String, usize>,
    work_items: &mut [WorkItemSummary],
) -> Result<(), StoreError> {
    let mut statement = connection
        .prepare(
            "SELECT work_item_id, capability
             FROM board_work_item_required_capabilities
             ORDER BY work_item_id, capability",
        )
        .map_err(StoreError::Sqlite)?;
    let requirements = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(StoreError::Sqlite)?;
    for requirement in requirements {
        let (work_item_id, capability) = requirement.map_err(StoreError::Sqlite)?;
        let index = positions
            .get(&work_item_id)
            .copied()
            .ok_or(StoreError::CorruptState(
                "capability requirement owner is missing",
            ))?;
        work_items[index].required_capabilities.push(capability);
    }
    Ok(())
}

fn load_required_capabilities(
    connection: &Connection,
    work_item_id: &str,
) -> Result<Vec<String>, StoreError> {
    let mut statement = connection
        .prepare(
            "SELECT capability
             FROM board_work_item_required_capabilities
             WHERE work_item_id = ?1
             ORDER BY capability",
        )
        .map_err(StoreError::Sqlite)?;
    let capabilities = statement
        .query_map([work_item_id], |row| row.get::<_, String>(0))
        .map_err(StoreError::Sqlite)?;
    capabilities
        .collect::<Result<Vec<_>, _>>()
        .map_err(StoreError::Sqlite)
}

fn load_execution_workspace(
    connection: &Connection,
    project_id: &str,
) -> Result<Option<ExecutionWorkspaceConnection>, StoreError> {
    let raw = connection
        .query_row(
            "SELECT project_id, kind, location
             FROM board_execution_workspaces
             WHERE project_id = ?1",
            [project_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .optional()
        .map_err(StoreError::Sqlite)?;
    raw.map(|(project_id, kind, location)| {
        let kind = ExecutionWorkspaceKind::try_from(kind.as_str())
            .map_err(|_| StoreError::CorruptState("unknown Execution workspace kind"))?;
        canonical_execution_workspace(&ExecutionWorkspaceConnection {
            project_id,
            kind,
            location,
        })
    })
    .transpose()
}

fn load_agent_profile(
    connection: &Connection,
    agent_profile_id: &str,
) -> Result<Option<AgentProfileSummary>, StoreError> {
    let Some(mut profile) = connection
        .query_row(
            "SELECT id, role, instruction_ref
             FROM board_agent_profiles WHERE id = ?1",
            [agent_profile_id],
            |row| {
                Ok(AgentProfileSummary {
                    id: row.get(0)?,
                    role: row.get(1)?,
                    capabilities: Vec::new(),
                    instruction_ref: row.get(2)?,
                    skill_refs: Vec::new(),
                })
            },
        )
        .optional()
        .map_err(StoreError::Sqlite)?
    else {
        return Ok(None);
    };
    let mut statement = connection
        .prepare(
            "SELECT capability
             FROM board_agent_profile_capabilities
             WHERE agent_profile_id = ?1
             ORDER BY capability",
        )
        .map_err(StoreError::Sqlite)?;
    let capabilities = statement
        .query_map([agent_profile_id], |row| row.get::<_, String>(0))
        .map_err(StoreError::Sqlite)?;
    profile.capabilities = capabilities
        .collect::<Result<Vec<_>, _>>()
        .map_err(StoreError::Sqlite)?;
    drop(statement);
    let mut statement = connection
        .prepare(
            "SELECT skill_ref
             FROM board_agent_profile_skills
             WHERE agent_profile_id = ?1
             ORDER BY skill_ref",
        )
        .map_err(StoreError::Sqlite)?;
    let skill_refs = statement
        .query_map([agent_profile_id], |row| row.get::<_, String>(0))
        .map_err(StoreError::Sqlite)?;
    profile.skill_refs = skill_refs
        .collect::<Result<Vec<_>, _>>()
        .map_err(StoreError::Sqlite)?;
    Ok(Some(profile))
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

fn parse_approval_requirement(value: &str) -> Result<ApprovalRequirement, StoreError> {
    ApprovalRequirement::try_from(value)
        .map_err(|_| StoreError::CorruptState("unknown Approval requirement"))
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

fn canonical_agent_profile(
    profile: &AgentProfileSummary,
) -> Result<AgentProfileSummary, StoreError> {
    validate_stable_identifier(&profile.id)?;
    validate_agent_role(&profile.role)?;
    if let Some(instruction_ref) = &profile.instruction_ref {
        validate_instruction_ref(instruction_ref)?;
    }
    let mut capabilities = profile.capabilities.clone();
    for capability in &capabilities {
        validate_capability(capability)?;
    }
    capabilities.sort();
    capabilities.dedup();
    let mut skill_refs = profile.skill_refs.clone();
    for skill_ref in &skill_refs {
        validate_stable_identifier(skill_ref)?;
    }
    skill_refs.sort();
    skill_refs.dedup();
    Ok(AgentProfileSummary {
        id: profile.id.clone(),
        role: profile.role.clone(),
        capabilities,
        instruction_ref: profile.instruction_ref.clone(),
        skill_refs,
    })
}

fn validate_stable_identifier(value: &str) -> Result<(), StoreError> {
    let length = value.chars().count();
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return Err(StoreError::InvalidRequest);
    };
    let last = value.chars().next_back().unwrap_or(first);
    if length > 64
        || !first.is_ascii_lowercase()
        || !last.is_ascii_alphanumeric()
        || !value.chars().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '-' | '_')
        })
    {
        return Err(StoreError::InvalidRequest);
    }
    Ok(())
}

fn validate_instruction_ref(value: &str) -> Result<(), StoreError> {
    let invalid_segment = value
        .split('/')
        .any(|segment| segment.is_empty() || matches!(segment, "." | ".."));
    if value.chars().count() > 512
        || value.trim() != value
        || value.starts_with('/')
        || value.contains(['\\', ':'])
        || value.chars().any(char::is_control)
        || invalid_segment
    {
        return Err(StoreError::InvalidRequest);
    }
    Ok(())
}

fn validate_agent_role(value: &str) -> Result<(), StoreError> {
    let length = value.chars().count();
    if length == 0 || length > 128 || value.trim() != value {
        return Err(StoreError::InvalidRequest);
    }
    Ok(())
}

fn canonical_agent_plan(plan: &AgentPlan) -> Result<AgentPlan, StoreError> {
    if let Some(agent_profile_id) = &plan.agent_profile_id {
        validate_id(agent_profile_id)?;
    }
    let mut required_capabilities = plan.required_capabilities.clone();
    for capability in &required_capabilities {
        validate_capability(capability)?;
    }
    required_capabilities.sort();
    required_capabilities.dedup();
    Ok(AgentPlan {
        agent_profile_id: plan.agent_profile_id.clone(),
        required_capabilities,
    })
}

fn validate_capability(value: &str) -> Result<(), StoreError> {
    let length = value.chars().count();
    if length == 0 || length > 64 || value.trim() != value {
        return Err(StoreError::InvalidRequest);
    }
    Ok(())
}

fn validate_blueprint_application(
    application: &BlueprintApplication,
    runtime_binding: &BlueprintRuntimeBinding,
) -> Result<(), StoreError> {
    for value in [
        &application.application_id,
        &application.blueprint_id,
        &application.revision_id,
        &application.entry_node_id,
        &application.project_id,
        &runtime_binding.application_id,
        &runtime_binding.project_id,
        &runtime_binding.agent_profile_id,
    ] {
        validate_stable_identifier(value)?;
    }
    validate_work_item_reference(&application.work_item_id)?;
    validate_work_item_reference(&runtime_binding.work_item_id)?;
    if application.application_id != runtime_binding.application_id
        || application.project_id != runtime_binding.project_id
        || application.work_item_id != runtime_binding.work_item_id
        || runtime_binding.execution_workspace.project_id != application.project_id
    {
        return Err(StoreError::InvalidRequest);
    }
    Ok(())
}

fn canonical_blueprint_note_pins(
    pins: &[BlueprintApproachNotePin],
) -> Result<Vec<BlueprintApproachNotePin>, StoreError> {
    let mut pins = pins.to_vec();
    for pin in &pins {
        pin.validate().map_err(|_| StoreError::InvalidRequest)?;
    }
    pins.sort_by(|left, right| left.approach_id.cmp(&right.approach_id));
    if pins
        .windows(2)
        .any(|pair| pair[0].approach_id == pair[1].approach_id)
    {
        return Err(StoreError::InvalidRequest);
    }
    Ok(pins)
}

fn validate_blueprint_application_facts(
    connection: &Connection,
    application: &BlueprintApplication,
    runtime_binding: &BlueprintRuntimeBinding,
    workspace: &ExecutionWorkspaceConnection,
    pins: &[BlueprintApproachNotePin],
) -> Result<(), StoreError> {
    let definition: String = connection
        .query_row(
            "SELECT definition_json
             FROM board_orchestration_blueprint_revisions
             WHERE blueprint_id = ?1 AND revision_id = ?2",
            params![application.blueprint_id, application.revision_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(StoreError::Sqlite)?
        .ok_or(StoreError::BlueprintRevisionNotFound)?;
    let revision: OrchestrationBlueprintRevision =
        serde_json::from_str(&definition).map_err(StoreError::Json)?;
    revision
        .validate()
        .map_err(|_| StoreError::CorruptState("invalid Orchestration Blueprint revision"))?;
    if revision.entry_node_id != application.entry_node_id {
        return Err(StoreError::ConcurrentChange);
    }
    let mut expected_approaches = revision
        .approach_ids()
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    expected_approaches.sort();
    expected_approaches.dedup();
    if expected_approaches
        != pins
            .iter()
            .map(|pin| pin.approach_id.clone())
            .collect::<Vec<_>>()
    {
        return Err(StoreError::InvalidRequest);
    }

    let (work_item_project_id, assigned_agent_profile_id) = connection
        .query_row(
            "SELECT project_id, agent_profile_id
             FROM board_work_items
             WHERE id = ?1",
            [&application.work_item_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .optional()
        .map_err(StoreError::Sqlite)?
        .ok_or(StoreError::WorkItemNotFound)?;
    if work_item_project_id != application.project_id {
        return Err(StoreError::WorkItemNotFound);
    }
    if assigned_agent_profile_id.as_deref() != Some(runtime_binding.agent_profile_id.as_str()) {
        return Err(StoreError::ConcurrentChange);
    }
    if load_agent_profile(connection, &runtime_binding.agent_profile_id)?.is_none() {
        return Err(StoreError::AgentProfileNotFound);
    }
    let current_workspace = load_execution_workspace(connection, &application.project_id)?
        .ok_or(StoreError::ExecutionWorkspaceNotFound)?;
    if current_workspace != *workspace {
        return Err(StoreError::ConcurrentChange);
    }
    Ok(())
}

fn load_blueprint_application_receipt(
    connection: &Connection,
    application_id: &str,
) -> Result<Option<BlueprintApplicationReceipt>, StoreError> {
    let raw = connection
        .query_row(
            "SELECT a.blueprint_id, a.revision_id, a.entry_node_id, a.project_id,
                    a.work_item_id, b.agent_profile_id, b.execution_workspace_kind,
                    b.execution_workspace_location, b.approach_notes_json
             FROM board_blueprint_applications AS a
             JOIN board_blueprint_runtime_bindings AS b
               ON b.application_id = a.application_id
             WHERE a.application_id = ?1",
            [application_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, String>(8)?,
                ))
            },
        )
        .optional()
        .map_err(StoreError::Sqlite)?;
    raw.map(
        |(
            blueprint_id,
            revision_id,
            entry_node_id,
            project_id,
            work_item_id,
            agent_profile_id,
            workspace_kind,
            workspace_location,
            pins_json,
        )| {
            let kind = ExecutionWorkspaceKind::try_from(workspace_kind.as_str())
                .map_err(|_| StoreError::CorruptState("unknown Execution workspace kind"))?;
            let execution_workspace =
                canonical_execution_workspace(&ExecutionWorkspaceConnection {
                    project_id: project_id.clone(),
                    kind,
                    location: workspace_location,
                })?;
            let pins: Vec<BlueprintApproachNotePin> =
                serde_json::from_str(&pins_json).map_err(StoreError::Json)?;
            let approach_notes = canonical_blueprint_note_pins(&pins)
                .map_err(|_| StoreError::CorruptState("invalid Blueprint Approach Note pins"))?;
            Ok(BlueprintApplicationReceipt {
                application: BlueprintApplication {
                    application_id: application_id.to_owned(),
                    blueprint_id,
                    revision_id,
                    entry_node_id,
                    project_id: project_id.clone(),
                    work_item_id: work_item_id.clone(),
                },
                runtime_binding: BlueprintRuntimeBinding {
                    application_id: application_id.to_owned(),
                    project_id,
                    work_item_id,
                    agent_profile_id,
                    execution_workspace,
                    approach_notes,
                },
                created: false,
            })
        },
    )
    .transpose()
}

fn canonical_execution_workspace(
    connection: &ExecutionWorkspaceConnection,
) -> Result<ExecutionWorkspaceConnection, StoreError> {
    validate_id(&connection.project_id)?;
    let location = match (connection.kind, &connection.location) {
        (ExecutionWorkspaceKind::BundledSample, None) => None,
        (ExecutionWorkspaceKind::BundledSample, Some(_))
        | (ExecutionWorkspaceKind::LocalDirectory, None) => return Err(StoreError::InvalidRequest),
        (ExecutionWorkspaceKind::LocalDirectory, Some(location)) => {
            let length = location.chars().count();
            if length == 0
                || length > 2048
                || location.trim() != location
                || location.chars().any(char::is_control)
                || !Path::new(location).is_absolute()
            {
                return Err(StoreError::InvalidRequest);
            }
            Some(location.clone())
        }
    };
    Ok(ExecutionWorkspaceConnection {
        project_id: connection.project_id.clone(),
        kind: connection.kind,
        location,
    })
}

fn validate_route_decision_request(request: &RouteDecisionRequest) -> Result<(), StoreError> {
    validate_stable_identifier(&request.decision_id)?;
    validate_stable_identifier(&request.project_id)?;
    validate_stable_identifier(&request.expected_current_node_id)?;
    validate_work_item_reference(&request.work_item_id)?;
    if let Some(proposed_route_id) = &request.proposed_route_id {
        validate_stable_identifier(proposed_route_id)?;
    }
    if request.evidence_refs.is_empty() || request.evidence_refs.len() > 32 {
        return Err(StoreError::InvalidRequest);
    }
    for evidence_ref in &request.evidence_refs {
        let length = evidence_ref.chars().count();
        if length == 0
            || length > 512
            || evidence_ref.trim() != evidence_ref
            || evidence_ref.chars().any(char::is_control)
        {
            return Err(StoreError::InvalidRequest);
        }
    }
    Ok(())
}

fn validate_graph_rewrite_proposal(proposal: &GraphRewriteProposal) -> Result<(), StoreError> {
    validate_stable_identifier(&proposal.proposal_id)?;
    validate_stable_identifier(&proposal.project_id)?;
    if proposal.source_binding.project_id != proposal.project_id
        || proposal.candidate_graph.graph_id != proposal.source_binding.graph_id
        || proposal.candidate_graph.revision_id == proposal.source_binding.revision_id
        || !proposal
            .candidate_graph
            .entries
            .iter()
            .any(|entry| entry.id == proposal.source_binding.entry_id)
    {
        return Err(StoreError::InvalidRequest);
    }
    proposal
        .candidate_graph
        .validate()
        .map_err(|_| StoreError::InvalidRequest)?;
    if !all_graph_nodes_are_reachable(&proposal.candidate_graph) {
        return Err(StoreError::InvalidRequest);
    }
    let rationale_length = proposal.rationale.chars().count();
    if rationale_length == 0
        || rationale_length > 1024
        || proposal.rationale.trim() != proposal.rationale
        || proposal.rationale.chars().any(char::is_control)
        || proposal.evidence_refs.is_empty()
        || proposal.evidence_refs.len() > 32
    {
        return Err(StoreError::InvalidRequest);
    }
    for evidence_ref in &proposal.evidence_refs {
        let length = evidence_ref.chars().count();
        if length == 0
            || length > 512
            || evidence_ref.trim() != evidence_ref
            || evidence_ref.chars().any(char::is_control)
        {
            return Err(StoreError::InvalidRequest);
        }
    }
    match &proposal.operation {
        GraphRewriteOperation::ReplaceAgentProfile {
            node_id,
            previous_agent_profile_id,
            replacement_agent_profile_id,
        } => {
            validate_stable_identifier(node_id)?;
            validate_stable_identifier(previous_agent_profile_id)?;
            validate_stable_identifier(replacement_agent_profile_id)?;
            if previous_agent_profile_id == replacement_agent_profile_id
                || !proposal.candidate_graph.nodes.iter().any(|node| {
                    node.id == *node_id
                        && matches!(
                            &node.kind,
                            ControlNodeKind::AgentLoop { agent_profile_id }
                                if agent_profile_id == replacement_agent_profile_id
                        )
                })
            {
                return Err(StoreError::InvalidRequest);
            }
        }
        GraphRewriteOperation::EditTopology {
            added_nodes,
            removed_node_ids,
            added_routes,
            removed_route_ids,
        } => {
            let edit_count = added_nodes.len()
                + removed_node_ids.len()
                + added_routes.len()
                + removed_route_ids.len();
            if edit_count == 0 || edit_count > 128 {
                return Err(StoreError::InvalidRequest);
            }
            for node in added_nodes {
                validate_stable_identifier(&node.id)?;
                if let ControlNodeKind::AgentLoop { agent_profile_id } = &node.kind {
                    validate_stable_identifier(agent_profile_id)?;
                }
            }
            for node_id in removed_node_ids {
                validate_stable_identifier(node_id)?;
            }
            for route in added_routes {
                validate_stable_identifier(&route.id)?;
                validate_stable_identifier(&route.source_node_id)?;
                validate_stable_identifier(&route.destination_node_id)?;
            }
            for route_id in removed_route_ids {
                validate_stable_identifier(route_id)?;
            }
        }
    }
    Ok(())
}

fn validate_graph_rewrite_relationships(
    connection: &Connection,
    proposal: &GraphRewriteProposal,
) -> Result<(), StoreError> {
    let source_graph = load_control_graph_revision(
        connection,
        &proposal.source_binding.graph_id,
        &proposal.source_binding.revision_id,
    )?;
    let mut expected_candidate = source_graph;
    expected_candidate
        .revision_id
        .clone_from(&proposal.candidate_graph.revision_id);
    match &proposal.operation {
        GraphRewriteOperation::ReplaceAgentProfile {
            node_id,
            previous_agent_profile_id,
            replacement_agent_profile_id,
        } => {
            let replacement_exists: bool = connection
                .query_row(
                    "SELECT EXISTS(
                       SELECT 1 FROM board_agent_profiles WHERE id = ?1
                     )",
                    [replacement_agent_profile_id],
                    |row| row.get(0),
                )
                .map_err(StoreError::Sqlite)?;
            if !replacement_exists {
                return Err(StoreError::AgentProfileNotFound);
            }
            let node = expected_candidate
                .nodes
                .iter_mut()
                .find(|node| node.id == *node_id)
                .ok_or(StoreError::InvalidRequest)?;
            let ControlNodeKind::AgentLoop { agent_profile_id } = &mut node.kind else {
                return Err(StoreError::InvalidRequest);
            };
            if agent_profile_id != previous_agent_profile_id {
                return Err(StoreError::InvalidRequest);
            }
            agent_profile_id.clone_from(replacement_agent_profile_id);
        }
        GraphRewriteOperation::EditTopology {
            added_nodes,
            removed_node_ids,
            added_routes,
            removed_route_ids,
        } => {
            for node in added_nodes {
                if let ControlNodeKind::AgentLoop { agent_profile_id } = &node.kind {
                    let profile_exists: bool = connection
                        .query_row(
                            "SELECT EXISTS(
                               SELECT 1 FROM board_agent_profiles WHERE id = ?1
                             )",
                            [agent_profile_id],
                            |row| row.get(0),
                        )
                        .map_err(StoreError::Sqlite)?;
                    if !profile_exists {
                        return Err(StoreError::AgentProfileNotFound);
                    }
                }
            }
            expected_candidate
                .routes
                .retain(|route| !removed_route_ids.contains(&route.id));
            expected_candidate
                .nodes
                .retain(|node| !removed_node_ids.contains(&node.id));
            expected_candidate.nodes.extend(added_nodes.iter().cloned());
            expected_candidate
                .routes
                .extend(added_routes.iter().cloned());
        }
    }
    if expected_candidate != proposal.candidate_graph {
        return Err(StoreError::InvalidRequest);
    }
    Ok(())
}

fn all_graph_nodes_are_reachable(graph: &ControlGraphRevision) -> bool {
    let mut reachable = graph
        .entries
        .iter()
        .map(|entry| entry.node_id.as_str())
        .collect::<HashSet<_>>();
    let mut queue = reachable.iter().copied().collect::<VecDeque<_>>();
    while let Some(source) = queue.pop_front() {
        for destination in graph
            .routes
            .iter()
            .filter(|route| route.source_node_id == source)
            .map(|route| route.destination_node_id.as_str())
        {
            if reachable.insert(destination) {
                queue.push_back(destination);
            }
        }
    }
    reachable.len() == graph.nodes.len()
}

fn publish_graph_rewrite_candidate(
    transaction: &rusqlite::Transaction<'_>,
    proposal: &GraphRewriteProposal,
    previous_binding: &ProjectGraphBinding,
) -> Result<ProjectGraphBinding, StoreError> {
    if previous_binding != &proposal.source_binding {
        return Err(StoreError::ConcurrentChange);
    }
    let candidate_exists: bool = transaction
        .query_row(
            "SELECT EXISTS(
               SELECT 1 FROM board_control_graph_revisions
               WHERE graph_id = ?1 AND revision_id = ?2
             )",
            params![
                proposal.candidate_graph.graph_id,
                proposal.candidate_graph.revision_id
            ],
            |row| row.get(0),
        )
        .map_err(StoreError::Sqlite)?;
    if candidate_exists {
        return Err(StoreError::GraphRevisionAlreadyExists);
    }
    let candidate_definition =
        serde_json::to_string(&proposal.candidate_graph).map_err(StoreError::Json)?;
    transaction
        .execute(
            "INSERT INTO board_control_graph_revisions
               (graph_id, revision_id, definition_json)
             VALUES (?1, ?2, ?3)",
            params![
                proposal.candidate_graph.graph_id,
                proposal.candidate_graph.revision_id,
                candidate_definition
            ],
        )
        .map_err(StoreError::Sqlite)?;
    let mut resulting_binding = previous_binding.clone();
    resulting_binding
        .revision_id
        .clone_from(&proposal.candidate_graph.revision_id);
    let changed = transaction
        .execute(
            "UPDATE board_project_graph_bindings
             SET revision_id = ?1
             WHERE project_id = ?2
               AND graph_id = ?3
               AND revision_id = ?4
               AND entry_id = ?5",
            params![
                resulting_binding.revision_id,
                proposal.source_binding.project_id,
                proposal.source_binding.graph_id,
                proposal.source_binding.revision_id,
                proposal.source_binding.entry_id
            ],
        )
        .map_err(StoreError::Sqlite)?;
    if changed != 1 {
        return Err(StoreError::ConcurrentChange);
    }
    Ok(resulting_binding)
}

fn validate_work_item_reference(work_item_id: &str) -> Result<(), StoreError> {
    let length = work_item_id.chars().count();
    if length == 0
        || length > 128
        || work_item_id.trim() != work_item_id
        || work_item_id.chars().any(char::is_control)
    {
        return Err(StoreError::InvalidRequest);
    }
    Ok(())
}

fn load_control_graph_revision(
    connection: &Connection,
    graph_id: &str,
    revision_id: &str,
) -> Result<ControlGraphRevision, StoreError> {
    let definition: Option<String> = connection
        .query_row(
            "SELECT definition_json
             FROM board_control_graph_revisions
             WHERE graph_id = ?1 AND revision_id = ?2",
            params![graph_id, revision_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(StoreError::Sqlite)?;
    let Some(definition) = definition else {
        return Err(StoreError::GraphRevisionNotFound);
    };
    let graph: ControlGraphRevision =
        serde_json::from_str(&definition).map_err(StoreError::Json)?;
    graph
        .validate()
        .map_err(|_| StoreError::CorruptState("invalid Control graph revision"))?;
    Ok(graph)
}

fn resolve_agent_loop_target(
    connection: &Connection,
    position: &WorkItemGraphPosition,
) -> Result<AgentLoopExecutionTarget, StoreError> {
    let graph = load_control_graph_revision(connection, &position.graph_id, &position.revision_id)?;
    let target = graph
        .agent_loop_target(&position.current_node_id)
        .map_err(|_| StoreError::ControlNodeNotExecutable)?;
    let profile_exists: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM board_agent_profiles WHERE id = ?1)",
            [&target.agent_profile_id],
            |row| row.get(0),
        )
        .map_err(StoreError::Sqlite)?;
    if !profile_exists {
        return Err(StoreError::CorruptState(
            "Control node Agent profile is missing",
        ));
    }
    Ok(target)
}

fn initial_graph_position(
    connection: &Connection,
    project_id: &str,
    work_item_id: &str,
) -> Result<WorkItemGraphPosition, StoreError> {
    let binding: Option<ProjectGraphBinding> = connection
        .query_row(
            "SELECT project_id, graph_id, revision_id, entry_id
             FROM board_project_graph_bindings
             WHERE project_id = ?1",
            [project_id],
            |row| {
                Ok(ProjectGraphBinding {
                    project_id: row.get(0)?,
                    graph_id: row.get(1)?,
                    revision_id: row.get(2)?,
                    entry_id: row.get(3)?,
                })
            },
        )
        .optional()
        .map_err(StoreError::Sqlite)?;
    let Some(binding) = binding else {
        return Err(StoreError::ProjectGraphBindingNotFound);
    };
    let graph = load_control_graph_revision(connection, &binding.graph_id, &binding.revision_id)?;
    binding
        .validate_against(&graph)
        .map_err(|_| StoreError::CorruptState("invalid Project graph binding"))?;
    let entry = graph
        .entries
        .iter()
        .find(|entry| entry.id == binding.entry_id)
        .ok_or(StoreError::CorruptState("Graph entry is missing"))?;
    Ok(WorkItemGraphPosition {
        project_id: project_id.to_owned(),
        work_item_id: work_item_id.to_owned(),
        graph_id: binding.graph_id,
        revision_id: binding.revision_id,
        entry_id: binding.entry_id,
        current_node_id: entry.node_id.clone(),
    })
}

fn position_after(decision: &RouteDecision) -> WorkItemGraphPosition {
    WorkItemGraphPosition {
        project_id: decision.project_id.clone(),
        work_item_id: decision.work_item_id.clone(),
        graph_id: decision.graph_id.clone(),
        revision_id: decision.revision_id.clone(),
        entry_id: decision.entry_id.clone(),
        current_node_id: decision.next_node_id.clone(),
    }
}

fn persist_route_decision(
    connection: &Connection,
    decision: &RouteDecision,
    resulting_position: &WorkItemGraphPosition,
) -> Result<(), StoreError> {
    let signal_json = serde_json::to_string(&decision.signal).map_err(StoreError::Json)?;
    let evidence_refs_json =
        serde_json::to_string(&decision.evidence_refs).map_err(StoreError::Json)?;
    connection
        .execute(
            "INSERT INTO board_route_decisions (
               decision_id, project_id, work_item_id, graph_id, revision_id, entry_id,
               source_node_id, signal_json, proposed_route_id, route_id, next_node_id,
               evidence_refs_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                decision.decision_id,
                decision.project_id,
                decision.work_item_id,
                decision.graph_id,
                decision.revision_id,
                decision.entry_id,
                decision.source_node_id,
                signal_json,
                decision.proposed_route_id,
                decision.route_id,
                decision.next_node_id,
                evidence_refs_json
            ],
        )
        .map_err(StoreError::Sqlite)?;
    connection
        .execute(
            "INSERT INTO board_work_item_graph_positions (
               work_item_id, project_id, graph_id, revision_id, entry_id, current_node_id
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(work_item_id) DO UPDATE SET current_node_id = excluded.current_node_id",
            params![
                resulting_position.work_item_id,
                resulting_position.project_id,
                resulting_position.graph_id,
                resulting_position.revision_id,
                resulting_position.entry_id,
                resulting_position.current_node_id
            ],
        )
        .map_err(StoreError::Sqlite)?;
    Ok(())
}

fn route_decision_matches_request(
    decision: &RouteDecision,
    request: &RouteDecisionRequest,
) -> bool {
    decision.decision_id == request.decision_id
        && decision.project_id == request.project_id
        && decision.work_item_id == request.work_item_id
        && decision.source_node_id == request.expected_current_node_id
        && decision.signal == request.signal
        && decision.proposed_route_id == request.proposed_route_id
        && decision.evidence_refs == request.evidence_refs
}

fn route_decision_select_sql(suffix: &str) -> String {
    format!(
        "SELECT decision_id, project_id, work_item_id, graph_id, revision_id, entry_id,
                source_node_id, signal_json, proposed_route_id, route_id, next_node_id,
                evidence_refs_json
         FROM board_route_decisions {suffix}"
    )
}

struct RawRouteDecision {
    decision_id: String,
    project_id: String,
    work_item_id: String,
    graph_id: String,
    revision_id: String,
    entry_id: String,
    source_node_id: String,
    signal_json: String,
    proposed_route_id: Option<String>,
    route_id: String,
    next_node_id: String,
    evidence_refs_json: String,
}

fn raw_route_decision(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawRouteDecision> {
    Ok(RawRouteDecision {
        decision_id: row.get(0)?,
        project_id: row.get(1)?,
        work_item_id: row.get(2)?,
        graph_id: row.get(3)?,
        revision_id: row.get(4)?,
        entry_id: row.get(5)?,
        source_node_id: row.get(6)?,
        signal_json: row.get(7)?,
        proposed_route_id: row.get(8)?,
        route_id: row.get(9)?,
        next_node_id: row.get(10)?,
        evidence_refs_json: row.get(11)?,
    })
}

fn decode_route_decision(raw: RawRouteDecision) -> Result<RouteDecision, StoreError> {
    let signal = serde_json::from_str(&raw.signal_json).map_err(StoreError::Json)?;
    let evidence_refs = serde_json::from_str(&raw.evidence_refs_json).map_err(StoreError::Json)?;
    Ok(RouteDecision {
        decision_id: raw.decision_id,
        project_id: raw.project_id,
        work_item_id: raw.work_item_id,
        graph_id: raw.graph_id,
        revision_id: raw.revision_id,
        entry_id: raw.entry_id,
        source_node_id: raw.source_node_id,
        signal,
        proposed_route_id: raw.proposed_route_id,
        route_id: raw.route_id,
        next_node_id: raw.next_node_id,
        evidence_refs,
    })
}

fn graph_position_select_sql(suffix: &str) -> String {
    format!(
        "SELECT project_id, work_item_id, graph_id, revision_id, entry_id, current_node_id
         FROM board_work_item_graph_positions {suffix}"
    )
}

fn graph_position_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkItemGraphPosition> {
    Ok(WorkItemGraphPosition {
        project_id: row.get(0)?,
        work_item_id: row.get(1)?,
        graph_id: row.get(2)?,
        revision_id: row.get(3)?,
        entry_id: row.get(4)?,
        current_node_id: row.get(5)?,
    })
}

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("request did not satisfy the Board contract")]
    InvalidRequest,
    #[error("Board project was not found")]
    ProjectNotFound,
    #[error("a Board project with this ID already exists")]
    ProjectAlreadyExists,
    #[error("a Control graph revision with this identity already exists")]
    GraphRevisionAlreadyExists,
    #[error("an Orchestration Blueprint revision with this identity already exists")]
    BlueprintRevisionAlreadyExists,
    #[error("the requested Orchestration Blueprint revision was not found")]
    BlueprintRevisionNotFound,
    #[error("a Blueprint Application with this identity already exists")]
    BlueprintApplicationAlreadyExists,
    #[error("a Portfolio orchestration revision with this identity already exists")]
    PortfolioOrchestrationRevisionAlreadyExists,
    #[error("a Graph rewrite proposal with this identity already exists")]
    GraphRewriteProposalAlreadyExists,
    #[error("the requested Graph rewrite proposal was not found")]
    GraphRewriteProposalNotFound,
    #[error("the Graph rewrite proposal already has a final decision")]
    GraphRewriteProposalAlreadyDecided,
    #[error("the requested Control graph revision was not found")]
    GraphRevisionNotFound,
    #[error("the Board project does not have a selected Control graph")]
    ProjectGraphBindingNotFound,
    #[error("a Route decision with this identity already exists")]
    RouteDecisionAlreadyExists,
    #[error("Safe Autopilot could not select the proposed Control route")]
    RouteNotSelected,
    #[error("the Work item does not have a pinned Control graph position")]
    WorkItemGraphPositionNotFound,
    #[error("the current Control node is not an Agent Loop execution stage")]
    ControlNodeNotExecutable,
    #[error("Work item was not found in the requested project")]
    WorkItemNotFound,
    #[error("Agent profile was not found")]
    AgentProfileNotFound,
    #[error("Execution workspace was not found")]
    ExecutionWorkspaceNotFound,
    #[error("an Agent profile with this ID already exists")]
    AgentProfileAlreadyExists,
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
    #[error("the Board record changed before the operation was applied")]
    ConcurrentChange,
    #[error("could not create the Board data directory")]
    CreateDirectory(#[source] std::io::Error),
    #[error("Board storage operation failed")]
    Sqlite(#[source] rusqlite::Error),
    #[error("Board graph serialization failed")]
    Json(#[source] serde_json::Error),
    #[error("Board storage is corrupt: {0}")]
    CorruptState(&'static str),
}

#[cfg(test)]
mod tests {
    use gareji_board_domain::{
        CheckpointOutcome, CheckpointSource, ControlGraphRevision, ControlNode, ControlNodeKind,
        ControlRoute, ControlSignal, GraphAnchor, GraphCanvasLayout, GraphCanvasNodePosition,
        GraphEntry, ProgressActivity, ProjectGraphBinding, ProjectGraphBindingSaveRequest,
        RouteDecisionRequest,
    };

    use super::*;

    #[test]
    fn project_can_select_a_persisted_graph_revision_and_entry() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();
        store.seed_sample_if_empty().unwrap();
        let graph = reviewed_graph();

        assert!(store.save_control_graph_revision(&graph).unwrap());
        assert!(!store.save_control_graph_revision(&graph).unwrap());

        let target = ProjectGraphBinding {
            project_id: "gareji-board".to_owned(),
            graph_id: "reviewed".to_owned(),
            revision_id: "v1".to_owned(),
            entry_id: "standard".to_owned(),
        };
        let receipt = store
            .save_project_graph_binding(&ProjectGraphBindingSaveRequest {
                expected: None,
                target: target.clone(),
            })
            .unwrap();

        assert!(receipt.changed);
        assert_eq!(receipt.previous, None);
        assert_eq!(receipt.resulting, target);
        assert_eq!(store.load_control_graph_revisions().unwrap(), vec![graph]);
        assert_eq!(
            store.load_project_graph_bindings().unwrap(),
            vec![receipt.resulting]
        );
    }

    #[test]
    fn graph_canvas_layout_is_saved_independently_from_graph_revisions() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();
        let mut layout = GraphCanvasLayout {
            graph_id: "reviewed".to_owned(),
            positions: vec![GraphCanvasNodePosition {
                node_id: "verify".to_owned(),
                x: 480.0,
                y: 220.0,
            }],
        };

        store.save_graph_canvas_layout(&layout).unwrap();
        assert_eq!(
            store.load_graph_canvas_layouts().unwrap(),
            vec![layout.clone()]
        );

        layout.positions[0].x = 640.0;
        store.save_graph_canvas_layout(&layout).unwrap();
        assert_eq!(store.load_graph_canvas_layouts().unwrap(), vec![layout]);
        assert!(store.load_control_graph_revisions().unwrap().is_empty());
    }

    #[test]
    fn project_route_history_is_ordered_and_does_not_cross_project_boundaries() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();
        store.seed_sample_if_empty().unwrap();
        store.ensure_builtin_control_graphs().unwrap();
        for project_id in ["gareji-board", "gareji-core"] {
            store
                .save_project_graph_binding(&ProjectGraphBindingSaveRequest {
                    expected: None,
                    target: ProjectGraphBinding {
                        project_id: project_id.to_owned(),
                        graph_id: "direct".to_owned(),
                        revision_id: "v1".to_owned(),
                        entry_id: "standard".to_owned(),
                    },
                })
                .unwrap();
        }
        store
            .record_route_decision(&RouteDecisionRequest {
                decision_id: "board-1-implemented".to_owned(),
                project_id: "gareji-board".to_owned(),
                work_item_id: "BOARD-1".to_owned(),
                expected_current_node_id: "implement".to_owned(),
                signal: ControlSignal::Succeeded,
                proposed_route_id: None,
                evidence_refs: vec!["checkpoint:board-1".to_owned()],
            })
            .unwrap();
        store
            .record_route_decision(&RouteDecisionRequest {
                decision_id: "core-1-implemented".to_owned(),
                project_id: "gareji-core".to_owned(),
                work_item_id: "CORE-1".to_owned(),
                expected_current_node_id: "implement".to_owned(),
                signal: ControlSignal::Succeeded,
                proposed_route_id: None,
                evidence_refs: vec!["checkpoint:core-1".to_owned()],
            })
            .unwrap();
        store
            .record_route_decision(&RouteDecisionRequest {
                decision_id: "board-1-verified".to_owned(),
                project_id: "gareji-board".to_owned(),
                work_item_id: "BOARD-1".to_owned(),
                expected_current_node_id: "verify".to_owned(),
                signal: ControlSignal::Passed,
                proposed_route_id: None,
                evidence_refs: vec!["test:cargo-test-workspace".to_owned()],
            })
            .unwrap();

        let history = store.load_project_route_decisions("gareji-board").unwrap();

        assert_eq!(history.len(), 2);
        assert_eq!(history[0].decision_id, "board-1-implemented");
        assert_eq!(history[1].decision_id, "board-1-verified");
        assert!(
            history
                .iter()
                .all(|decision| decision.project_id == "gareji-board")
        );
    }

    #[test]
    fn builtin_control_graphs_are_available_idempotently() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();

        assert_eq!(store.ensure_builtin_control_graphs().unwrap(), 3);
        assert_eq!(store.ensure_builtin_control_graphs().unwrap(), 0);

        let identities = store
            .load_control_graph_revisions()
            .unwrap()
            .into_iter()
            .map(|graph| (graph.graph_id, graph.revision_id))
            .collect::<Vec<_>>();
        assert_eq!(
            identities,
            vec![
                ("direct".to_owned(), "v1".to_owned()),
                ("high-risk".to_owned(), "v1".to_owned()),
                ("reviewed".to_owned(), "v1".to_owned()),
            ]
        );
    }

    #[test]
    fn builtin_portfolio_orchestration_is_available_idempotently() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();

        assert_eq!(store.ensure_builtin_portfolio_orchestrations().unwrap(), 1);
        assert_eq!(store.ensure_builtin_portfolio_orchestrations().unwrap(), 0);

        let revisions = store.load_portfolio_orchestration_revisions().unwrap();
        assert_eq!(revisions.len(), 1);
        assert_eq!(revisions[0].orchestration_id, "managed-products");
        assert_eq!(revisions[0].revision_id, "v2");
        assert_eq!(revisions[0].nodes.len(), 3);
        assert_eq!(revisions[0].routes.len(), 4);
        assert!(matches!(
            revisions[0].nodes[0].kind,
            PortfolioNodeKind::ProjectSelector {
                selector: PortfolioProjectSelector::AllManaged
            }
        ));
    }

    #[test]
    fn portfolio_ticks_are_atomic_and_append_only() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();
        store.ensure_builtin_portfolio_orchestrations().unwrap();
        let first_run = PortfolioRun {
            run_id: "run-1".to_owned(),
            orchestration_id: "managed-products".to_owned(),
            revision_id: "v2".to_owned(),
            current_node_id: "summary".to_owned(),
            status: PortfolioRunStatus::Active,
            completed_steps: 1,
            next_tick_at_epoch_seconds: Some(3_600),
        };
        let first_step = PortfolioRunStep {
            run_id: "run-1".to_owned(),
            sequence: 1,
            node_id: "select-project".to_owned(),
            signal: Some(PortfolioSignal::NoCandidate),
            destination_node_id: Some("summary".to_owned()),
            selected_project_id: None,
            recorded_at_epoch_seconds: 0,
        };

        store
            .record_portfolio_tick(None, &first_run, &first_step)
            .unwrap();
        assert_eq!(
            store
                .load_latest_portfolio_run("managed-products", "v2")
                .unwrap(),
            Some(first_run.clone())
        );
        assert_eq!(
            store.load_portfolio_run_steps("run-1").unwrap(),
            vec![first_step]
        );

        let stale = PortfolioRun {
            completed_steps: 2,
            ..first_run.clone()
        };
        assert!(matches!(
            store.record_portfolio_tick(
                None,
                &stale,
                &PortfolioRunStep {
                    run_id: "run-1".to_owned(),
                    sequence: 2,
                    node_id: "summary".to_owned(),
                    signal: Some(PortfolioSignal::Completed),
                    destination_node_id: Some("finish".to_owned()),
                    selected_project_id: None,
                    recorded_at_epoch_seconds: 1,
                }
            ),
            Err(StoreError::ConcurrentChange)
        ));
    }

    #[test]
    fn portfolio_restart_compares_the_latest_completed_run() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();
        store.ensure_builtin_portfolio_orchestrations().unwrap();
        let completed = PortfolioRun {
            run_id: "completed-run".to_owned(),
            orchestration_id: "managed-products".to_owned(),
            revision_id: "v2".to_owned(),
            current_node_id: "finish".to_owned(),
            status: PortfolioRunStatus::Completed,
            completed_steps: 1,
            next_tick_at_epoch_seconds: Some(3_600),
        };
        let completed_step = PortfolioRunStep {
            run_id: completed.run_id.clone(),
            sequence: 1,
            node_id: "finish".to_owned(),
            signal: None,
            destination_node_id: None,
            selected_project_id: None,
            recorded_at_epoch_seconds: 0,
        };
        store
            .record_portfolio_tick(None, &completed, &completed_step)
            .unwrap();

        let restarted = PortfolioRun {
            run_id: "restarted-run".to_owned(),
            orchestration_id: completed.orchestration_id.clone(),
            revision_id: completed.revision_id.clone(),
            current_node_id: "summary".to_owned(),
            status: PortfolioRunStatus::Active,
            completed_steps: 1,
            next_tick_at_epoch_seconds: Some(7_200),
        };
        let restarted_step = PortfolioRunStep {
            run_id: restarted.run_id.clone(),
            sequence: 1,
            node_id: "select-project".to_owned(),
            signal: Some(PortfolioSignal::NoCandidate),
            destination_node_id: Some("summary".to_owned()),
            selected_project_id: None,
            recorded_at_epoch_seconds: 3_600,
        };
        store
            .record_portfolio_tick(Some(&completed), &restarted, &restarted_step)
            .unwrap();

        let rival = PortfolioRun {
            run_id: "rival-run".to_owned(),
            ..restarted.clone()
        };
        let rival_step = PortfolioRunStep {
            run_id: rival.run_id.clone(),
            ..restarted_step
        };
        assert!(matches!(
            store.record_portfolio_tick(Some(&completed), &rival, &rival_step),
            Err(StoreError::ConcurrentChange)
        ));
        assert_eq!(
            store
                .load_latest_portfolio_run("managed-products", "v2")
                .unwrap(),
            Some(restarted)
        );
    }

    #[test]
    fn portfolio_schedule_control_pauses_and_resumes_without_changing_the_run() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();
        store.ensure_builtin_portfolio_orchestrations().unwrap();
        let revision = store
            .load_portfolio_orchestration_revisions()
            .unwrap()
            .remove(0);
        let active = store.load_portfolio_schedule_control(&revision).unwrap();
        assert!(active.automatic_ticks_enabled);

        let paused = PortfolioScheduleControl {
            automatic_ticks_enabled: false,
            ..active.clone()
        };
        let receipt = store
            .save_portfolio_schedule_control(&PortfolioScheduleControlSaveRequest {
                expected: active.clone(),
                target: paused.clone(),
            })
            .unwrap();
        assert!(receipt.changed);
        assert_eq!(
            store.load_portfolio_schedule_control(&revision).unwrap(),
            paused
        );

        let resumed = PortfolioScheduleControl {
            automatic_ticks_enabled: true,
            ..paused.clone()
        };
        store
            .save_portfolio_schedule_control(&PortfolioScheduleControlSaveRequest {
                expected: paused.clone(),
                target: resumed.clone(),
            })
            .unwrap();
        assert_eq!(
            store.load_portfolio_schedule_control(&revision).unwrap(),
            resumed
        );

        assert!(matches!(
            store.save_portfolio_schedule_control(&PortfolioScheduleControlSaveRequest {
                expected: paused.clone(),
                target: paused,
            }),
            Err(StoreError::ConcurrentChange)
        ));
    }

    #[test]
    fn builtin_orchestration_blueprint_is_portable_and_idempotent() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();

        assert_eq!(store.ensure_builtin_orchestration_blueprints().unwrap(), 1);
        assert_eq!(store.ensure_builtin_orchestration_blueprints().unwrap(), 0);

        let revisions = store.load_orchestration_blueprint_revisions().unwrap();
        assert_eq!(revisions.len(), 1);
        assert_eq!(revisions[0].blueprint_id, "evidence-first");
        assert_eq!(revisions[0].scope, BlueprintScope::Project);
        assert_eq!(revisions[0].approach_ids(), vec!["evidence-first"]);
    }

    #[test]
    fn accepted_blueprint_application_pins_runtime_facts_without_starting_work() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();
        store.seed_sample_if_empty().unwrap();
        store.ensure_builtin_orchestration_blueprints().unwrap();
        let application = BlueprintApplication {
            application_id: "application-core-2".to_owned(),
            blueprint_id: "evidence-first".to_owned(),
            revision_id: "v1".to_owned(),
            entry_node_id: "approach".to_owned(),
            project_id: "gareji-core".to_owned(),
            work_item_id: "CORE-2".to_owned(),
        };
        let runtime_binding = BlueprintRuntimeBinding {
            application_id: application.application_id.clone(),
            project_id: application.project_id.clone(),
            work_item_id: application.work_item_id.clone(),
            agent_profile_id: "implementer".to_owned(),
            execution_workspace: ExecutionWorkspaceConnection {
                project_id: application.project_id.clone(),
                kind: ExecutionWorkspaceKind::BundledSample,
                location: None,
            },
            approach_notes: vec![BlueprintApproachNotePin {
                approach_id: "evidence-first".to_owned(),
                absolute_path: std::env::current_dir()
                    .unwrap()
                    .join("evidence-first.md")
                    .to_string_lossy()
                    .into_owned(),
                fingerprint: "a".repeat(64),
            }],
        };

        let first = store
            .accept_blueprint_application(&application, &runtime_binding)
            .unwrap();
        store
            .connection
            .execute(
                "UPDATE board_work_items SET agent_profile_id = 'reviewer' WHERE id = 'CORE-2'",
                [],
            )
            .unwrap();
        let retry = store
            .accept_blueprint_application(&application, &runtime_binding)
            .unwrap();

        assert!(first.created);
        assert!(!retry.created);
        assert_eq!(store.load_blueprint_applications().unwrap(), vec![retry]);
        assert_eq!(
            store
                .assess_active_work("gareji-core", "CORE-2")
                .unwrap()
                .state,
            WorkItemState::Todo
        );
    }

    #[test]
    fn route_decision_pins_the_work_item_and_advances_its_current_node() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();
        store.seed_sample_if_empty().unwrap();
        store.ensure_builtin_control_graphs().unwrap();
        store
            .save_project_graph_binding(&ProjectGraphBindingSaveRequest {
                expected: None,
                target: ProjectGraphBinding {
                    project_id: "gareji-board".to_owned(),
                    graph_id: "direct".to_owned(),
                    revision_id: "v1".to_owned(),
                    entry_id: "standard".to_owned(),
                },
            })
            .unwrap();
        let request = RouteDecisionRequest {
            decision_id: "board-1-implementation-complete".to_owned(),
            project_id: "gareji-board".to_owned(),
            work_item_id: "BOARD-1".to_owned(),
            expected_current_node_id: "implement".to_owned(),
            signal: ControlSignal::Succeeded,
            proposed_route_id: Some("implementation-complete".to_owned()),
            evidence_refs: vec!["checkpoint:cp-board-1".to_owned()],
        };

        let first = store.record_route_decision(&request).unwrap();
        let retry = store.record_route_decision(&request).unwrap();

        assert!(first.recorded);
        assert!(!retry.recorded);
        assert_eq!(first.decision.route_id, "implementation-complete");
        assert_eq!(first.resulting_position.current_node_id, "verify");
        assert_eq!(first.resulting_position.graph_id, "direct");
        assert_eq!(
            store.load_route_decisions("BOARD-1").unwrap(),
            vec![first.decision]
        );
        assert_eq!(
            store.load_work_item_graph_positions().unwrap(),
            vec![first.resulting_position]
        );
    }

    #[test]
    fn current_agent_loop_becomes_a_concrete_runner_target() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();
        store.seed_sample_if_empty().unwrap();
        store.ensure_builtin_control_graphs().unwrap();
        store
            .save_project_graph_binding(&ProjectGraphBindingSaveRequest {
                expected: None,
                target: ProjectGraphBinding {
                    project_id: "gareji-board".to_owned(),
                    graph_id: "reviewed".to_owned(),
                    revision_id: "v1".to_owned(),
                    entry_id: "standard".to_owned(),
                },
            })
            .unwrap();
        store
            .record_route_decision(&RouteDecisionRequest {
                decision_id: "board-1-research-complete".to_owned(),
                project_id: "gareji-board".to_owned(),
                work_item_id: "BOARD-1".to_owned(),
                expected_current_node_id: "research".to_owned(),
                signal: ControlSignal::Succeeded,
                proposed_route_id: Some("research-complete".to_owned()),
                evidence_refs: vec!["checkpoint:cp-research".to_owned()],
            })
            .unwrap();

        let target = store.load_agent_loop_execution_target("BOARD-1").unwrap();

        assert_eq!(target.graph_id, "reviewed");
        assert_eq!(target.revision_id, "v1");
        assert_eq!(target.node_id, "implement");
        assert_eq!(target.agent_profile_id, "implementer");
    }

    #[test]
    fn runner_preparation_pins_the_entry_agent_loop_before_the_first_decision() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();
        store.seed_sample_if_empty().unwrap();
        store.ensure_builtin_control_graphs().unwrap();
        store
            .save_project_graph_binding(&ProjectGraphBindingSaveRequest {
                expected: None,
                target: ProjectGraphBinding {
                    project_id: "gareji-board".to_owned(),
                    graph_id: "reviewed".to_owned(),
                    revision_id: "v1".to_owned(),
                    entry_id: "standard".to_owned(),
                },
            })
            .unwrap();

        let target = store
            .prepare_agent_loop_execution_target("gareji-board", "BOARD-1")
            .unwrap();

        assert_eq!(target.node_id, "research");
        assert_eq!(target.agent_profile_id, "researcher");
        assert_eq!(
            store.load_work_item_graph_positions().unwrap(),
            vec![WorkItemGraphPosition {
                project_id: "gareji-board".to_owned(),
                work_item_id: "BOARD-1".to_owned(),
                graph_id: "reviewed".to_owned(),
                revision_id: "v1".to_owned(),
                entry_id: "standard".to_owned(),
                current_node_id: "research".to_owned(),
            }]
        );
    }

    #[test]
    fn runner_preparation_does_not_pin_ineligible_work() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();
        store.seed_sample_if_empty().unwrap();
        store.ensure_builtin_control_graphs().unwrap();
        store
            .save_project_graph_binding(&ProjectGraphBindingSaveRequest {
                expected: None,
                target: ProjectGraphBinding {
                    project_id: "zettelkasten-plugin".to_owned(),
                    graph_id: "direct".to_owned(),
                    revision_id: "v1".to_owned(),
                    entry_id: "standard".to_owned(),
                },
            })
            .unwrap();

        assert!(matches!(
            store.prepare_agent_loop_execution_target("zettelkasten-plugin", "ZETTEL-1"),
            Err(StoreError::InvalidRequest)
        ));
        assert!(store.load_work_item_graph_positions().unwrap().is_empty());
    }

    fn reviewed_graph() -> ControlGraphRevision {
        ControlGraphRevision {
            graph_id: "reviewed".to_owned(),
            revision_id: "v1".to_owned(),
            entries: vec![GraphEntry {
                id: "standard".to_owned(),
                node_id: "implement".to_owned(),
            }],
            nodes: vec![
                ControlNode {
                    id: "implement".to_owned(),
                    kind: ControlNodeKind::AgentLoop {
                        agent_profile_id: "implementer".to_owned(),
                    },
                },
                ControlNode {
                    id: "verify".to_owned(),
                    kind: ControlNodeKind::Audit,
                },
                ControlNode {
                    id: "finish".to_owned(),
                    kind: ControlNodeKind::Terminal,
                },
            ],
            routes: vec![
                ControlRoute {
                    id: "implementation-complete".to_owned(),
                    source_node_id: "implement".to_owned(),
                    destination_node_id: "verify".to_owned(),
                    signal: ControlSignal::Succeeded,
                },
                ControlRoute {
                    id: "verification-passed".to_owned(),
                    source_node_id: "verify".to_owned(),
                    destination_node_id: "finish".to_owned(),
                    signal: ControlSignal::Passed,
                },
            ],
            anchors: vec![GraphAnchor {
                id: "tests-ran".to_owned(),
                description: "Verification is grounded in tests that actually ran".to_owned(),
            }],
        }
    }

    #[test]
    fn sample_seed_is_idempotent_and_loads_one_query_read_model() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();

        assert!(store.seed_sample_if_empty().unwrap());
        assert!(!store.seed_sample_if_empty().unwrap());

        let snapshot = store.load_portfolio().unwrap();
        assert_eq!(snapshot.projects.len(), 3);
        assert_eq!(snapshot.active_runs(), 1);
        assert_eq!(snapshot.blocked_items(), 1);

        let work_items = store.load_work_items().unwrap();
        let profiles = store.load_agent_profiles().unwrap();
        let execution_workspaces = store.load_execution_workspaces().unwrap();
        assert_eq!(execution_workspaces.len(), 3);
        assert!(execution_workspaces.iter().all(|connection| {
            connection.kind == ExecutionWorkspaceKind::BundledSample
                && connection.location.is_none()
        }));
        assert_eq!(profiles.len(), 4);
        assert_eq!(profiles[0].id, "implementer");
        assert_eq!(
            profiles[0].capabilities,
            vec!["implementation".to_owned(), "testing".to_owned()]
        );
        assert_eq!(
            profiles[0].instruction_ref.as_deref(),
            Some("agents/implementer/AGENT.md")
        );
        assert_eq!(
            profiles[0].skill_refs,
            vec![
                "implement-bounded-work-item".to_owned(),
                "write-project-handoff".to_owned()
            ]
        );
        let approval = work_items
            .iter()
            .find(|work_item| work_item.id == "BOARD-2")
            .unwrap();
        assert_eq!(approval.approval_requirement, ApprovalRequirement::Explicit);
        let dependent = work_items
            .iter()
            .find(|work_item| work_item.id == "BOARD-3")
            .unwrap();
        assert_eq!(dependent.dependency_ids, vec!["CORE-1".to_owned()]);
        let candidate = work_items
            .iter()
            .find(|work_item| work_item.id == "CORE-2")
            .unwrap();
        assert_eq!(candidate.agent_profile_id.as_deref(), Some("implementer"));
        assert_eq!(
            candidate.required_capabilities,
            vec!["implementation".to_owned()]
        );
    }

    #[test]
    fn disposable_demo_labels_are_explicit_idempotent_and_keep_stable_ids() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();
        store.seed_sample_if_empty().unwrap();

        let before = store.load_portfolio().unwrap();
        assert_eq!(
            before
                .projects
                .iter()
                .find(|project| project.id == "gareji-core")
                .unwrap()
                .name,
            "Gareji Core"
        );

        store.apply_demo_project_labels().unwrap();
        store.apply_demo_project_labels().unwrap();

        let after = store.load_portfolio().unwrap();
        for (project_id, expected_name) in [
            ("gareji-board", "Gareji Board · Sample"),
            ("gareji-core", "Gareji Core · Sample"),
            ("zettelkasten-plugin", "Sample Knowledge Plugin"),
        ] {
            let project = after
                .projects
                .iter()
                .find(|project| project.id == project_id)
                .unwrap();
            assert_eq!(project.name, expected_name);
        }
    }

    #[test]
    fn execution_workspace_save_is_atomic_idempotent_and_rejects_stale_observations() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();
        store.seed_sample_if_empty().unwrap();
        let existing = store
            .load_execution_workspaces()
            .unwrap()
            .into_iter()
            .find(|connection| connection.project_id == "gareji-board")
            .unwrap();
        let target = ExecutionWorkspaceConnection {
            project_id: "gareji-board".to_owned(),
            kind: ExecutionWorkspaceKind::LocalDirectory,
            location: Some(std::env::temp_dir().display().to_string()),
        };

        let saved = store
            .save_execution_workspace(&ExecutionWorkspaceSaveRequest {
                expected: Some(existing.clone()),
                target: target.clone(),
            })
            .unwrap();
        assert!(saved.changed);
        assert_eq!(saved.previous, Some(existing));
        assert_eq!(saved.resulting, target);

        let unchanged = store
            .save_execution_workspace(&ExecutionWorkspaceSaveRequest {
                expected: Some(saved.resulting.clone()),
                target: saved.resulting.clone(),
            })
            .unwrap();
        assert!(!unchanged.changed);
        assert!(matches!(
            store.save_execution_workspace(&ExecutionWorkspaceSaveRequest {
                expected: Some(ExecutionWorkspaceConnection {
                    project_id: "gareji-board".to_owned(),
                    kind: ExecutionWorkspaceKind::BundledSample,
                    location: None,
                }),
                target,
            }),
            Err(StoreError::ConcurrentChange)
        ));
    }

    #[test]
    fn execution_workspace_save_validates_project_and_connection_shape() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();
        store.seed_sample_if_empty().unwrap();
        assert!(matches!(
            store.save_execution_workspace(&ExecutionWorkspaceSaveRequest {
                expected: None,
                target: ExecutionWorkspaceConnection {
                    project_id: "missing-project".to_owned(),
                    kind: ExecutionWorkspaceKind::BundledSample,
                    location: None,
                },
            }),
            Err(StoreError::ProjectNotFound)
        ));
        assert!(matches!(
            store.save_execution_workspace(&ExecutionWorkspaceSaveRequest {
                expected: Some(ExecutionWorkspaceConnection {
                    project_id: "gareji-core".to_owned(),
                    kind: ExecutionWorkspaceKind::BundledSample,
                    location: None,
                }),
                target: ExecutionWorkspaceConnection {
                    project_id: "gareji-core".to_owned(),
                    kind: ExecutionWorkspaceKind::BundledSample,
                    location: Some("not-allowed".to_owned()),
                },
            }),
            Err(StoreError::InvalidRequest)
        ));
    }

    #[test]
    fn project_creation_adds_an_idle_project_and_its_first_workspace_atomically() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();
        let request = ProjectCreateRequest {
            id: "existing-product".to_owned(),
            name: "Existing product".to_owned(),
            execution_cap: 2,
            execution_workspace: ExecutionWorkspaceConnection {
                project_id: "existing-product".to_owned(),
                kind: ExecutionWorkspaceKind::LocalDirectory,
                location: Some(std::env::temp_dir().display().to_string()),
            },
        };

        let receipt = store.create_project(&request).unwrap();
        assert_eq!(receipt.project.id, "existing-product");
        assert_eq!(receipt.project.health, ProjectHealth::Idle);
        assert_eq!(receipt.project.execution_cap, 2);
        assert_eq!(receipt.project.work_items, WorkItemCounts::default());
        assert_eq!(receipt.execution_workspace, request.execution_workspace);
        assert_eq!(
            store.load_portfolio().unwrap().projects,
            vec![receipt.project.clone()]
        );
        assert_eq!(
            store.load_execution_workspaces().unwrap(),
            vec![request.execution_workspace.clone()]
        );

        assert!(matches!(
            store.create_project(&request),
            Err(StoreError::ProjectAlreadyExists)
        ));
        assert_eq!(store.load_portfolio().unwrap().projects.len(), 1);
        assert_eq!(store.load_execution_workspaces().unwrap().len(), 1);
    }

    #[test]
    fn project_creation_rejects_invalid_identity_capacity_and_connection_shapes() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();
        let location = std::env::temp_dir().display().to_string();
        let valid = ProjectCreateRequest {
            id: "existing-product".to_owned(),
            name: "Existing product".to_owned(),
            execution_cap: 1,
            execution_workspace: ExecutionWorkspaceConnection {
                project_id: "existing-product".to_owned(),
                kind: ExecutionWorkspaceKind::LocalDirectory,
                location: Some(location.clone()),
            },
        };
        let invalid_id = ProjectCreateRequest {
            id: "Existing Product".to_owned(),
            ..valid.clone()
        };
        assert!(matches!(
            store.create_project(&invalid_id),
            Err(StoreError::InvalidRequest)
        ));
        let invalid_capacity = ProjectCreateRequest {
            execution_cap: 0,
            ..valid.clone()
        };
        assert!(matches!(
            store.create_project(&invalid_capacity),
            Err(StoreError::InvalidRequest)
        ));
        let mismatched_connection = ProjectCreateRequest {
            execution_workspace: ExecutionWorkspaceConnection {
                project_id: "another-project".to_owned(),
                kind: ExecutionWorkspaceKind::LocalDirectory,
                location: Some(location),
            },
            ..valid
        };
        assert!(matches!(
            store.create_project(&mismatched_connection),
            Err(StoreError::InvalidRequest)
        ));
        assert!(store.load_portfolio().unwrap().projects.is_empty());
        assert!(store.load_execution_workspaces().unwrap().is_empty());
    }

    #[test]
    fn direct_work_item_creation_uses_safe_defaults_and_rejects_duplicates() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();
        store.seed_sample_if_empty().unwrap();
        let request = WorkItemCreateRequest {
            project_id: "gareji-core".to_owned(),
            work_item_id: "CORE-3".to_owned(),
            title: "Connect an existing project".to_owned(),
            priority: 3,
        };

        let receipt = store.create_work_item(&request).unwrap();
        assert_eq!(receipt.work_item.id, "CORE-3");
        assert_eq!(receipt.work_item.project_id, "gareji-core");
        assert_eq!(receipt.work_item.state, WorkItemState::Todo);
        assert_eq!(receipt.work_item.priority, 3);
        assert_eq!(
            receipt.work_item.approval_requirement,
            ApprovalRequirement::None
        );
        assert!(receipt.work_item.dependency_ids.is_empty());
        assert!(receipt.work_item.agent_profile_id.is_none());
        assert!(receipt.work_item.required_capabilities.is_empty());
        assert!(
            store
                .load_work_items()
                .unwrap()
                .contains(&receipt.work_item)
        );

        assert!(matches!(
            store.create_work_item(&request),
            Err(StoreError::WorkItemAlreadyExists)
        ));
        assert!(matches!(
            store.create_work_item(&WorkItemCreateRequest {
                priority: 0,
                ..request
            }),
            Err(StoreError::InvalidRequest)
        ));
    }

    #[test]
    fn agent_plan_update_is_atomic_idempotent_and_rejects_stale_observations() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();
        store.seed_sample_if_empty().unwrap();
        let previous = AgentPlan {
            agent_profile_id: Some("implementer".to_owned()),
            required_capabilities: vec!["implementation".to_owned()],
        };
        let target = AgentPlan {
            agent_profile_id: Some("release-checker".to_owned()),
            required_capabilities: vec![
                "testing".to_owned(),
                "release".to_owned(),
                "testing".to_owned(),
            ],
        };
        let request = AgentPlanUpdateRequest {
            project_id: "gareji-core".to_owned(),
            work_item_id: "CORE-2".to_owned(),
            expected: previous.clone(),
            target,
        };

        let receipt = store.update_agent_plan(&request).unwrap();
        assert!(receipt.changed);
        assert_eq!(receipt.previous, previous);
        assert_eq!(
            receipt.resulting,
            AgentPlan {
                agent_profile_id: Some("release-checker".to_owned()),
                required_capabilities: vec!["release".to_owned(), "testing".to_owned()],
            }
        );
        assert!(matches!(
            store.update_agent_plan(&AgentPlanUpdateRequest {
                target: AgentPlan {
                    agent_profile_id: Some("reviewer".to_owned()),
                    required_capabilities: vec!["review".to_owned()],
                },
                ..request.clone()
            }),
            Err(StoreError::ConcurrentChange)
        ));

        let stored = store
            .load_work_items()
            .unwrap()
            .into_iter()
            .find(|work_item| work_item.id == "CORE-2")
            .unwrap();
        assert_eq!(stored.agent_plan(), receipt.resulting);
        let no_change = store
            .update_agent_plan(&AgentPlanUpdateRequest {
                expected: stored.agent_plan(),
                target: stored.agent_plan(),
                ..request
            })
            .unwrap();
        assert!(!no_change.changed);
    }

    #[test]
    fn agent_plan_update_validates_profile_and_project_relationship() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();
        store.seed_sample_if_empty().unwrap();
        let current = store
            .load_work_items()
            .unwrap()
            .into_iter()
            .find(|work_item| work_item.id == "CORE-2")
            .unwrap()
            .agent_plan();

        assert!(matches!(
            store.update_agent_plan(&AgentPlanUpdateRequest {
                project_id: "gareji-core".to_owned(),
                work_item_id: "CORE-2".to_owned(),
                expected: current.clone(),
                target: AgentPlan {
                    agent_profile_id: Some("missing".to_owned()),
                    required_capabilities: Vec::new(),
                },
            }),
            Err(StoreError::AgentProfileNotFound)
        ));
        assert!(matches!(
            store.update_agent_plan(&AgentPlanUpdateRequest {
                project_id: "gareji-board".to_owned(),
                work_item_id: "CORE-2".to_owned(),
                expected: current.clone(),
                target: current,
            }),
            Err(StoreError::WorkItemNotFound)
        ));
    }

    #[test]
    fn agent_profile_save_creates_updates_and_rejects_stale_observations() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();
        store.seed_sample_if_empty().unwrap();
        let create = AgentProfileSaveRequest {
            expected: None,
            target: AgentProfileSummary {
                id: "qa-specialist".to_owned(),
                role: "QA specialist".to_owned(),
                capabilities: vec![
                    "testing".to_owned(),
                    "evidence".to_owned(),
                    "testing".to_owned(),
                ],
                instruction_ref: Some("agents/qa/AGENT.md".to_owned()),
                skill_refs: vec![
                    "review-work-item".to_owned(),
                    "evidence-summary".to_owned(),
                    "review-work-item".to_owned(),
                ],
            },
        };

        let created = store.save_agent_profile(&create).unwrap();
        assert!(created.changed);
        assert_eq!(created.previous, None);
        assert_eq!(
            created.resulting.capabilities,
            vec!["evidence".to_owned(), "testing".to_owned()]
        );
        assert_eq!(
            created.resulting.skill_refs,
            vec!["evidence-summary".to_owned(), "review-work-item".to_owned()]
        );
        let unchanged = store
            .save_agent_profile(&AgentProfileSaveRequest {
                expected: Some(created.resulting.clone()),
                target: created.resulting.clone(),
            })
            .unwrap();
        assert!(!unchanged.changed);

        let edited_target = AgentProfileSummary {
            role: "Quality reviewer".to_owned(),
            capabilities: vec!["review".to_owned(), "testing".to_owned()],
            ..created.resulting.clone()
        };
        let edited = store
            .save_agent_profile(&AgentProfileSaveRequest {
                expected: Some(created.resulting.clone()),
                target: edited_target.clone(),
            })
            .unwrap();
        assert!(edited.changed);
        assert_eq!(edited.resulting, edited_target);
        assert!(matches!(
            store.save_agent_profile(&AgentProfileSaveRequest {
                expected: Some(created.resulting),
                target: AgentProfileSummary {
                    role: "Stale edit".to_owned(),
                    ..edited_target
                },
            }),
            Err(StoreError::ConcurrentChange)
        ));
    }

    #[test]
    fn agent_profile_save_preserves_stable_identity_and_unique_creation() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();
        store.seed_sample_if_empty().unwrap();
        let existing = store
            .load_agent_profiles()
            .unwrap()
            .into_iter()
            .find(|profile| profile.id == "implementer")
            .unwrap();

        assert!(matches!(
            store.save_agent_profile(&AgentProfileSaveRequest {
                expected: None,
                target: existing.clone(),
            }),
            Err(StoreError::AgentProfileAlreadyExists)
        ));
        assert!(matches!(
            store.save_agent_profile(&AgentProfileSaveRequest {
                expected: Some(existing.clone()),
                target: AgentProfileSummary {
                    id: "renamed".to_owned(),
                    ..existing.clone()
                },
            }),
            Err(StoreError::InvalidRequest)
        ));
        assert!(matches!(
            store.save_agent_profile(&AgentProfileSaveRequest {
                expected: None,
                target: AgentProfileSummary {
                    id: "Invalid ID".to_owned(),
                    role: "Invalid".to_owned(),
                    capabilities: Vec::new(),
                    instruction_ref: None,
                    skill_refs: Vec::new(),
                },
            }),
            Err(StoreError::InvalidRequest)
        ));
        for instruction_ref in ["../AGENT.md", "/agents/AGENT.md", "agents\\AGENT.md"] {
            assert!(matches!(
                store.save_agent_profile(&AgentProfileSaveRequest {
                    expected: None,
                    target: AgentProfileSummary {
                        id: "invalid-instructions".to_owned(),
                        role: "Invalid instructions".to_owned(),
                        capabilities: Vec::new(),
                        instruction_ref: Some(instruction_ref.to_owned()),
                        skill_refs: Vec::new(),
                    },
                }),
                Err(StoreError::InvalidRequest)
            ));
        }
        assert!(matches!(
            store.save_agent_profile(&AgentProfileSaveRequest {
                expected: None,
                target: AgentProfileSummary {
                    id: "invalid-skill".to_owned(),
                    role: "Invalid Skill".to_owned(),
                    capabilities: Vec::new(),
                    instruction_ref: None,
                    skill_refs: vec!["Remote Skill".to_owned()],
                },
            }),
            Err(StoreError::InvalidRequest)
        ));
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
        assert_eq!(work_item.priority, 100);
        assert_eq!(work_item.approval_requirement, ApprovalRequirement::None);
        assert!(work_item.dependency_ids.is_empty());
        assert_eq!(work_item.agent_profile_id, None);
        assert!(work_item.required_capabilities.is_empty());

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
        assert_current_execution_workspace_schema(&store);
        let priority: i64 = store
            .connection
            .query_row(
                "SELECT priority FROM board_work_items WHERE id = 'CORE-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(priority, 100);
        let approval_requirement: String = store
            .connection
            .query_row(
                "SELECT approval_requirement FROM board_work_items WHERE id = 'CORE-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(approval_requirement, "none");
        let agent_profile_id: Option<String> = store
            .connection
            .query_row(
                "SELECT agent_profile_id FROM board_work_items WHERE id = 'CORE-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(agent_profile_id, None);
        let dependency_table_exists: bool = store
            .connection
            .query_row(
                "SELECT EXISTS(
                   SELECT 1 FROM sqlite_master
                   WHERE type = 'table' AND name = 'board_work_item_dependencies'
                 )",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(dependency_table_exists);
        let agent_profile_table_exists: bool = store
            .connection
            .query_row(
                "SELECT EXISTS(
                   SELECT 1 FROM sqlite_master
                   WHERE type = 'table' AND name = 'board_agent_profiles'
                 )",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(agent_profile_table_exists);
        assert_agent_behavior_schema(&store);
    }

    fn assert_current_execution_workspace_schema(store: &SqliteBoardStore) {
        let version: i64 = store
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 15);
        let table_exists: bool = store
            .connection
            .query_row(
                "SELECT EXISTS(
                   SELECT 1 FROM sqlite_master
                   WHERE type = 'table' AND name = 'board_execution_workspaces'
                 )",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(table_exists);
        for table in [
            "board_control_graph_revisions",
            "board_project_graph_bindings",
            "board_graph_canvas_layouts",
            "board_work_item_graph_positions",
            "board_route_decisions",
            "board_graph_rewrite_proposals",
        ] {
            let exists: bool = store
                .connection
                .query_row(
                    "SELECT EXISTS(
                       SELECT 1 FROM sqlite_master
                       WHERE type = 'table' AND name = ?1
                     )",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(exists, "missing {table}");
        }
    }

    #[test]
    fn opening_a_v6_database_adds_agent_behavior_references() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE board_agent_profiles (
                   id TEXT PRIMARY KEY,
                   role TEXT NOT NULL
                 );
                 INSERT INTO board_agent_profiles VALUES ('reviewer', 'Reviewer');
                 PRAGMA user_version = 6;",
            )
            .unwrap();

        let store = SqliteBoardStore::from_connection(connection).unwrap();
        let profiles = store.load_agent_profiles().unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].instruction_ref, None);
        assert!(profiles[0].skill_refs.is_empty());
        assert_agent_behavior_schema(&store);
    }

    fn assert_agent_behavior_schema(store: &SqliteBoardStore) {
        assert!(
            table_column_exists(&store.connection, "board_agent_profiles", "instruction_ref")
                .unwrap()
        );
        let skill_table_exists: bool = store
            .connection
            .query_row(
                "SELECT EXISTS(
                   SELECT 1 FROM sqlite_master
                   WHERE type = 'table' AND name = 'board_agent_profile_skills'
                 )",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(skill_table_exists);
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

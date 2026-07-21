use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use gareji_board_core::{BoardRunController, CoreProgressReader, MarkdownApproachNoteReader};
use gareji_board_domain::{
    ActivityTimeline, AgentPlan, AgentPlanUpdateRequest, AgentProfileSaveRequest,
    AgentProfileSummary, ApproachNoteManifest, AttachmentRequest, AttachmentTarget,
    AutopilotStopReason, CandidateSkipReason, ControlGraphRevision, ExecutionWorkspaceConnection,
    ExecutionWorkspaceKind, GraphCanvasLayout, NoCandidateReason, OrchestrationBlueprintRevision,
    PortfolioOrchestrationRevision, ProjectCreateRequest as DomainProjectRequest,
    ProjectGraphBinding, ProjectGraphBindingSaveRequest, ReconciliationDecision,
    ReconciliationRequest, SafeAutopilotOutcome, SafeAutopilotPreview,
    WorkItemCreateRequest as DomainWorkItemRequest, WorkItemState, WorkItemSummary,
    WorkItemTransitionRequest,
};
use gareji_board_store::{SqliteBoardStore, default_board_database_path};
use serde::{Deserialize, Serialize};

const PREVIEW_GLOBAL_CONCURRENCY_CAP: u32 = 2;

/// Deep desktop Module used by every Tauri command.
///
/// Its Interface is deliberately small: callers receive one bounded read model
/// and submit explicit human intents. SQL, local paths, Runner construction, and
/// domain validation stay inside the implementation.
pub struct BoardModule;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopSnapshot {
    pub projects: Vec<ProjectView>,
    pub work_items: Vec<WorkItemView>,
    pub agent_profiles: Vec<AgentProfileView>,
    pub execution_workspaces: Vec<ExecutionWorkspaceView>,
    pub control_graphs: Vec<ControlGraphRevision>,
    pub graph_canvas_layouts: Vec<GraphCanvasLayout>,
    pub project_graph_bindings: Vec<ProjectGraphBinding>,
    pub orchestration_blueprints: Vec<OrchestrationBlueprintRevision>,
    pub portfolio_orchestrations: Vec<PortfolioOrchestrationRevision>,
    pub approach_notes: Vec<ApproachNoteView>,
    pub activity: Vec<ActivityView>,
    pub metrics: MetricsView,
    pub storage_label: String,
    pub demo_workspace: bool,
    pub warning: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricsView {
    pub projects: usize,
    pub active_runs: u32,
    pub blocked_items: u32,
    pub delivery_issues: u32,
    pub inbox_items: u32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectView {
    pub id: String,
    pub name: String,
    pub health: String,
    pub execution_cap: u32,
    pub active_runs: u32,
    pub total_items: u32,
    pub todo_items: u32,
    pub blocked_items: u32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkItemView {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub priority: u32,
    pub state: String,
    pub approval_requirement: String,
    pub dependency_ids: Vec<String>,
    pub agent_profile_id: Option<String>,
    pub required_capabilities: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentProfileView {
    pub id: String,
    pub role: String,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionWorkspaceView {
    pub project_id: String,
    pub kind: String,
    pub location: Option<String>,
    pub display_name: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApproachNoteView {
    pub approach_id: String,
    pub title: String,
    pub filename: String,
    pub fingerprint: String,
    pub risk: String,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityView {
    pub checkpoint_id: String,
    pub recorded_at: String,
    pub project_id: String,
    pub work_item_id: Option<String>,
    pub source: String,
    pub outcome: String,
    pub summary: String,
    pub recommended_state: Option<String>,
    pub delivery_issues: usize,
    pub attachment: Option<String>,
    pub reconciliation: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewResult {
    pub status: String,
    pub summary: String,
    pub candidate: Option<PreviewCandidate>,
    pub skipped: Vec<SkippedCandidate>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewCandidate {
    pub work_item_id: String,
    pub title: String,
    pub project_name: String,
    pub agent_role: String,
    pub capacity: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedCandidate {
    pub work_item_id: String,
    pub reason: String,
}

#[derive(Debug, Serialize)]
pub struct ActionResult {
    pub message: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransitionRequest {
    pub project_id: String,
    pub work_item_id: String,
    pub expected_state: String,
    pub target_state: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateWorkItemRequest {
    pub project_id: String,
    pub work_item_id: String,
    pub title: String,
    pub priority: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateProjectRequest {
    pub project_id: String,
    pub name: String,
    pub execution_cap: u32,
    pub workspace_location: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BindGraphRequest {
    pub expected: Option<ProjectGraphBinding>,
    pub target: ProjectGraphBinding,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartAgentLoopRequest {
    pub work_item_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateAgentPlanRequest {
    pub project_id: String,
    pub work_item_id: String,
    pub expected_agent_profile_id: Option<String>,
    pub target_agent_profile_id: Option<String>,
    pub required_capabilities: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveAgentProfileRequest {
    pub profile_id: String,
    pub role: String,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconcileActivityRequest {
    pub checkpoint_id: String,
    pub project_id: String,
    pub work_item_id: String,
    pub recommended_state: String,
    pub decision: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_field_names)]
pub struct AttachActivityRequest {
    pub checkpoint_id: String,
    pub project_id: String,
    pub work_item_id: String,
}

impl BoardModule {
    pub fn load() -> Result<DesktopSnapshot, String> {
        let database_path = board_database_path();
        let mut store = open_store()?;
        prepare_store(&mut store)?;

        let portfolio = store.load_portfolio().map_err(display_error)?;
        let work_items = store.load_work_items().map_err(display_error)?;
        let agent_profiles = store.load_agent_profiles().map_err(display_error)?;
        let execution_workspaces = store.load_execution_workspaces().map_err(display_error)?;
        let control_graphs = store
            .load_control_graph_revisions()
            .map_err(display_error)?;
        let graph_canvas_layouts = store.load_graph_canvas_layouts().map_err(display_error)?;
        let project_graph_bindings = store.load_project_graph_bindings().map_err(display_error)?;
        let orchestration_blueprints = store
            .load_orchestration_blueprint_revisions()
            .map_err(display_error)?;
        let portfolio_orchestrations = store
            .load_portfolio_orchestration_revisions()
            .map_err(display_error)?;
        let (approach_notes, approach_warning) = load_approach_notes();

        let project_ids = portfolio
            .projects
            .iter()
            .map(|project| project.id.clone())
            .collect::<Vec<_>>();
        let (mut activity, core_warning) = CoreProgressReader::from_environment()
            .load_portfolio_activity(&project_ids)
            .map_or_else(
                |error| (ActivityTimeline::default(), Some(error.to_string())),
                |loaded| {
                    let warning = (loaded.skipped_projects > 0).then(|| {
                        format!(
                            "{} project(s) are not connected for progress history.",
                            loaded.skipped_projects
                        )
                    });
                    (loaded.timeline, warning)
                },
            );
        let hydration_warning = store
            .hydrate_activity(&mut activity)
            .err()
            .map(|error| format!("Board activity state: {error}"));
        let delivery_issues = activity.delivery_issues();
        let inbox_items = activity.inbox_count();

        let metrics = MetricsView {
            projects: portfolio.projects.len(),
            active_runs: portfolio.active_runs(),
            blocked_items: portfolio.blocked_items(),
            delivery_issues,
            inbox_items,
        };

        Ok(DesktopSnapshot {
            projects: portfolio.projects.iter().map(ProjectView::from).collect(),
            work_items: work_items.iter().map(WorkItemView::from).collect(),
            agent_profiles: agent_profiles.iter().map(AgentProfileView::from).collect(),
            execution_workspaces: execution_workspaces
                .iter()
                .map(ExecutionWorkspaceView::from)
                .collect(),
            control_graphs,
            graph_canvas_layouts,
            project_graph_bindings,
            orchestration_blueprints,
            portfolio_orchestrations,
            approach_notes: approach_notes.iter().map(ApproachNoteView::from).collect(),
            activity: activity.activities.iter().map(ActivityView::from).collect(),
            metrics,
            storage_label: database_path.display().to_string(),
            demo_workspace: is_demo_workspace(),
            warning: join_warnings([approach_warning, core_warning, hydration_warning]),
        })
    }

    pub fn preview_safe_autopilot() -> Result<PreviewResult, String> {
        let mut store = open_store()?;
        prepare_store(&mut store)?;
        let portfolio = store.load_portfolio().map_err(display_error)?;
        let work_items = store.load_work_items().map_err(display_error)?;
        let agent_profiles = store.load_agent_profiles().map_err(display_error)?;
        let preview = SafeAutopilotPreview::evaluate(
            &portfolio,
            &work_items,
            &agent_profiles,
            PREVIEW_GLOBAL_CONCURRENCY_CAP,
        );
        Ok(PreviewResult::from_preview(preview))
    }

    pub fn transition_work_item(request: &TransitionRequest) -> Result<ActionResult, String> {
        let expected_state = parse_work_item_state(&request.expected_state)?;
        let target_state = parse_work_item_state(&request.target_state)?;
        let receipt = open_store()?
            .transition_work_item(&WorkItemTransitionRequest {
                project_id: request.project_id.clone(),
                work_item_id: request.work_item_id.clone(),
                expected_state,
                target_state,
            })
            .map_err(display_error)?;
        Ok(ActionResult {
            message: if receipt.changed {
                format!(
                    "{} moved from {} to {}.",
                    receipt.work_item_id,
                    receipt.previous_state.as_str(),
                    receipt.resulting_state.as_str()
                )
            } else {
                format!("{} was already up to date.", receipt.work_item_id)
            },
        })
    }

    pub fn create_work_item(request: &CreateWorkItemRequest) -> Result<ActionResult, String> {
        let receipt = open_store()?
            .create_work_item(&DomainWorkItemRequest {
                project_id: request.project_id.clone(),
                work_item_id: request.work_item_id.clone(),
                title: request.title.clone(),
                priority: request.priority,
            })
            .map_err(display_error)?;
        Ok(ActionResult {
            message: format!("{} was added to Todo.", receipt.work_item.id),
        })
    }

    pub fn create_project(request: &CreateProjectRequest) -> Result<ActionResult, String> {
        let location = canonical_workspace(&request.workspace_location)?;
        let target = ExecutionWorkspaceConnection {
            project_id: request.project_id.clone(),
            kind: ExecutionWorkspaceKind::LocalDirectory,
            location: Some(location.display().to_string()),
        };
        let receipt = open_store()?
            .create_project(&DomainProjectRequest {
                id: request.project_id.clone(),
                name: request.name.clone(),
                execution_cap: request.execution_cap,
                execution_workspace: target,
            })
            .map_err(display_error)?;
        Ok(ActionResult {
            message: format!("{} is now managed by Gareji Board.", receipt.project.name),
        })
    }

    pub fn save_graph_layout(layout: &GraphCanvasLayout) -> Result<ActionResult, String> {
        open_store()?
            .save_graph_canvas_layout(layout)
            .map_err(display_error)?;
        Ok(ActionResult {
            message: "Node positions saved.".to_owned(),
        })
    }

    pub fn publish_graph_revision(graph: &ControlGraphRevision) -> Result<ActionResult, String> {
        let inserted = open_store()?
            .save_control_graph_revision(graph)
            .map_err(display_error)?;
        Ok(ActionResult {
            message: if inserted {
                format!("Graph revision {} was published.", graph.revision_id)
            } else {
                format!("Graph revision {} already exists.", graph.revision_id)
            },
        })
    }

    pub fn bind_project_graph(request: &BindGraphRequest) -> Result<ActionResult, String> {
        let receipt = open_store()?
            .save_project_graph_binding(&ProjectGraphBindingSaveRequest {
                expected: request.expected.clone(),
                target: request.target.clone(),
            })
            .map_err(display_error)?;
        Ok(ActionResult {
            message: if receipt.changed {
                format!(
                    "{} now uses {} · {}.",
                    receipt.resulting.project_id,
                    receipt.resulting.graph_id,
                    receipt.resulting.revision_id
                )
            } else {
                "Project graph binding was already current.".to_owned()
            },
        })
    }

    pub fn publish_blueprint_revision(
        blueprint: &OrchestrationBlueprintRevision,
    ) -> Result<ActionResult, String> {
        let inserted = open_store()?
            .save_orchestration_blueprint_revision(blueprint)
            .map_err(display_error)?;
        Ok(ActionResult {
            message: if inserted {
                format!(
                    "Blueprint revision {} was published.",
                    blueprint.revision_id
                )
            } else {
                format!(
                    "Blueprint revision {} already exists.",
                    blueprint.revision_id
                )
            },
        })
    }

    pub fn start_agent_loop(request: &StartAgentLoopRequest) -> Result<ActionResult, String> {
        let mut store = open_store()?;
        let mut controller = BoardRunController::from_environment().map_err(display_error)?;
        let outcome = controller
            .run_current_agent_loop_with_defaults(&mut store, &request.work_item_id)
            .map_err(display_error)?;
        Ok(ActionResult {
            message: outcome.status_message(),
        })
    }

    pub fn update_agent_plan(request: &UpdateAgentPlanRequest) -> Result<ActionResult, String> {
        let receipt = open_store()?
            .update_agent_plan(&AgentPlanUpdateRequest {
                project_id: request.project_id.clone(),
                work_item_id: request.work_item_id.clone(),
                expected: AgentPlan {
                    agent_profile_id: request.expected_agent_profile_id.clone(),
                    required_capabilities: request.required_capabilities.clone(),
                },
                target: AgentPlan {
                    agent_profile_id: request.target_agent_profile_id.clone(),
                    required_capabilities: request.required_capabilities.clone(),
                },
            })
            .map_err(display_error)?;
        Ok(ActionResult {
            message: if receipt.changed {
                format!("Agent plan updated for {}.", receipt.work_item_id)
            } else {
                format!(
                    "Agent plan for {} was already current.",
                    receipt.work_item_id
                )
            },
        })
    }

    pub fn save_agent_profile(request: &SaveAgentProfileRequest) -> Result<ActionResult, String> {
        let target = AgentProfileSummary {
            id: request.profile_id.clone(),
            role: request.role.clone(),
            capabilities: request.capabilities.clone(),
            instruction_ref: None,
            skill_refs: Vec::new(),
        };
        let mut store = open_store()?;
        let existing = store
            .load_agent_profiles()
            .map_err(display_error)?
            .into_iter()
            .find(|profile| profile.id == target.id);
        let receipt = store
            .save_agent_profile(&AgentProfileSaveRequest {
                expected: existing,
                target,
            })
            .map_err(display_error)?;
        Ok(ActionResult {
            message: if receipt.changed {
                format!("Agent profile {} was saved.", receipt.resulting.id)
            } else {
                format!(
                    "Agent profile {} was already current.",
                    receipt.resulting.id
                )
            },
        })
    }

    pub fn reconcile_activity(request: &ReconcileActivityRequest) -> Result<ActionResult, String> {
        let recommended_state = parse_work_item_state(&request.recommended_state)?;
        let decision = match request.decision.as_str() {
            "accepted" => ReconciliationDecision::Accepted,
            "dismissed" => ReconciliationDecision::Dismissed,
            _ => return Err("Choose accept or dismiss for the recommendation.".to_owned()),
        };
        let receipt = open_store()?
            .reconcile_checkpoint(&ReconciliationRequest {
                checkpoint_id: request.checkpoint_id.clone(),
                project_id: request.project_id.clone(),
                work_item_id: request.work_item_id.clone(),
                recommended_state,
                decision,
            })
            .map_err(display_error)?;
        Ok(ActionResult {
            message: format!(
                "Recommendation for {} was {}.",
                receipt.checkpoint_id,
                receipt.reconciliation.decision.as_str()
            ),
        })
    }

    pub fn attach_activity(request: &AttachActivityRequest) -> Result<ActionResult, String> {
        let receipt = open_store()?
            .attach_checkpoint(&AttachmentRequest {
                checkpoint_id: request.checkpoint_id.clone(),
                project_id: request.project_id.clone(),
                checkpoint_work_item_id: None,
                target: AttachmentTarget::Existing {
                    work_item_id: request.work_item_id.clone(),
                },
            })
            .map_err(display_error)?;
        Ok(ActionResult {
            message: format!(
                "Checkpoint {} was attached to {}.",
                receipt.checkpoint_id, receipt.attachment.work_item_id
            ),
        })
    }
}

impl From<&gareji_board_domain::ProjectSummary> for ProjectView {
    fn from(project: &gareji_board_domain::ProjectSummary) -> Self {
        Self {
            id: project.id.clone(),
            name: project.name.clone(),
            health: project.health.as_str().to_owned(),
            execution_cap: project.execution_cap,
            active_runs: project.work_items.in_progress,
            total_items: project.work_items.total,
            todo_items: project.work_items.todo,
            blocked_items: project.work_items.blocked,
        }
    }
}

impl From<&WorkItemSummary> for WorkItemView {
    fn from(item: &WorkItemSummary) -> Self {
        Self {
            id: item.id.clone(),
            project_id: item.project_id.clone(),
            title: item.title.clone(),
            priority: item.priority,
            state: item.state.as_str().to_owned(),
            approval_requirement: item.approval_requirement.as_str().to_owned(),
            dependency_ids: item.dependency_ids.clone(),
            agent_profile_id: item.agent_profile_id.clone(),
            required_capabilities: item.required_capabilities.clone(),
        }
    }
}

impl From<&AgentProfileSummary> for AgentProfileView {
    fn from(profile: &AgentProfileSummary) -> Self {
        Self {
            id: profile.id.clone(),
            role: profile.role.clone(),
            capabilities: profile.capabilities.clone(),
        }
    }
}

impl From<&ExecutionWorkspaceConnection> for ExecutionWorkspaceView {
    fn from(connection: &ExecutionWorkspaceConnection) -> Self {
        let display_name = connection
            .location
            .as_deref()
            .and_then(portable_path_name)
            .unwrap_or("Bundled sample")
            .to_owned();
        Self {
            project_id: connection.project_id.clone(),
            kind: connection.kind.as_str().to_owned(),
            location: connection.location.clone(),
            display_name,
        }
    }
}

fn portable_path_name(path: &str) -> Option<&str> {
    path.rsplit(['/', '\\']).find(|segment| !segment.is_empty())
}

impl From<&ApproachNoteManifest> for ApproachNoteView {
    fn from(note: &ApproachNoteManifest) -> Self {
        Self {
            approach_id: note.approach_id.clone(),
            title: note.title.clone(),
            filename: Path::new(&note.absolute_path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("Approach note")
                .to_owned(),
            fingerprint: note.fingerprint.chars().take(12).collect(),
            risk: format!("{:?}", note.risk).to_lowercase(),
            inputs: note
                .inputs
                .iter()
                .map(|socket| format!("{socket:?}").to_lowercase())
                .collect(),
            outputs: note
                .outputs
                .iter()
                .map(|socket| format!("{socket:?}").to_lowercase())
                .collect(),
        }
    }
}

impl From<&gareji_board_domain::ProgressActivity> for ActivityView {
    fn from(activity: &gareji_board_domain::ProgressActivity) -> Self {
        Self {
            checkpoint_id: activity.checkpoint_id.clone(),
            recorded_at: activity.recorded_at.clone(),
            project_id: activity.project_id.clone(),
            work_item_id: activity.effective_work_item_id().map(ToOwned::to_owned),
            source: activity.source.label().to_owned(),
            outcome: activity.outcome.label().to_owned(),
            summary: activity.summary.clone(),
            recommended_state: activity
                .recommended_state
                .map(|state| state.as_str().to_owned()),
            delivery_issues: activity
                .deliveries
                .iter()
                .filter(|delivery| delivery.status.needs_attention())
                .count(),
            attachment: activity
                .attachment
                .as_ref()
                .map(|attachment| attachment.work_item_id.clone()),
            reconciliation: activity
                .reconciliation
                .as_ref()
                .map(|reconciliation| reconciliation.decision.as_str().to_owned()),
        }
    }
}

impl PreviewResult {
    fn from_preview(preview: SafeAutopilotPreview) -> Self {
        let skipped = preview
            .skipped
            .into_iter()
            .map(|candidate| SkippedCandidate {
                work_item_id: candidate.work_item.id,
                reason: skip_reason(&candidate.reason),
            })
            .collect();
        match preview.outcome {
            SafeAutopilotOutcome::Candidate(candidate) => Self {
                status: "candidate".to_owned(),
                summary: "One safe candidate is ready for an explicit start.".to_owned(),
                candidate: Some(PreviewCandidate {
                    work_item_id: candidate.work_item.id,
                    title: candidate.work_item.title,
                    project_name: candidate.project_name,
                    agent_role: candidate.agent_role,
                    capacity: format!(
                        "{}/{} running",
                        candidate.active_runs, candidate.execution_cap
                    ),
                }),
                skipped,
            },
            SafeAutopilotOutcome::NoCandidate(reason) => Self {
                status: "no_candidate".to_owned(),
                summary: no_candidate_reason(reason),
                candidate: None,
                skipped,
            },
            SafeAutopilotOutcome::Stop(reason) => Self {
                status: "stopped".to_owned(),
                summary: autopilot_stop_reason(&reason),
                candidate: None,
                skipped,
            },
        }
    }
}

fn open_store() -> Result<SqliteBoardStore, String> {
    SqliteBoardStore::open(board_database_path()).map_err(display_error)
}

fn prepare_store(store: &mut SqliteBoardStore) -> Result<(), String> {
    store.seed_sample_if_empty().map_err(display_error)?;
    store
        .ensure_builtin_control_graphs()
        .map_err(display_error)?;
    store
        .ensure_builtin_portfolio_orchestrations()
        .map_err(display_error)?;
    store
        .ensure_builtin_orchestration_blueprints()
        .map_err(display_error)?;
    Ok(())
}

fn board_database_path() -> PathBuf {
    env::var_os("GAREJI_BOARD_DB")
        .filter(|value| !value.is_empty())
        .map_or_else(default_board_database_path, PathBuf::from)
}

fn is_demo_workspace() -> bool {
    demo_workspace_from_marker(env::var_os("GAREJI_DEMO_EXECUTION_WORKSPACE"))
}

fn demo_workspace_from_marker(marker: Option<OsString>) -> bool {
    marker.is_some_and(|value| !value.is_empty())
}

fn canonical_workspace(value: &str) -> Result<PathBuf, String> {
    let path = Path::new(value.trim());
    if value.trim().is_empty() || !path.is_dir() {
        return Err("Choose an existing local project directory.".to_owned());
    }
    path.canonicalize()
        .map_err(|error| format!("Project directory could not be opened: {error}"))
}

fn load_approach_notes() -> (Vec<ApproachNoteManifest>, Option<String>) {
    let mut paths = env::var_os("GAREJI_APPROACH_NOTES").map_or_else(
        || discover_default_approach_notes(&env::current_dir().unwrap_or_default()),
        |value| env::split_paths(&value).collect::<Vec<_>>(),
    );
    paths.sort();
    paths.dedup();
    let reader = MarkdownApproachNoteReader::default();
    let mut notes = Vec::new();
    let mut errors = Vec::new();
    for path in paths {
        match reader.read(&path) {
            Ok(note) => notes.push(note),
            Err(error) => errors.push(format!("{}: {error}", path.display())),
        }
    }
    notes.sort_by(|left, right| left.approach_id.cmp(&right.approach_id));
    let warning = (!errors.is_empty())
        .then(|| format!("{} Approach Note(s) could not be loaded.", errors.len()));
    (notes, warning)
}

fn discover_default_approach_notes(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(root.join("approaches")) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| {
                    extension.eq_ignore_ascii_case("md")
                        || extension.eq_ignore_ascii_case("markdown")
                })
        })
        .collect()
}

fn parse_work_item_state(value: &str) -> Result<WorkItemState, String> {
    match value {
        "backlog" => Ok(WorkItemState::Backlog),
        "todo" => Ok(WorkItemState::Todo),
        "in_progress" => Ok(WorkItemState::InProgress),
        "in_review" => Ok(WorkItemState::InReview),
        "blocked" => Ok(WorkItemState::Blocked),
        "done" => Ok(WorkItemState::Done),
        "cancelled" => Ok(WorkItemState::Cancelled),
        _ => Err(format!("Unknown Work item state: {value}")),
    }
}

fn skip_reason(reason: &CandidateSkipReason) -> String {
    match reason {
        CandidateSkipReason::StateNotTodo(state) => format!("State is {}.", state.as_str()),
        CandidateSkipReason::ProjectAtCapacity {
            active_runs,
            execution_cap,
        } => format!("Project capacity is {active_runs}/{execution_cap}."),
        CandidateSkipReason::GlobalCapacityReached { .. } => {
            "Global execution capacity is full.".to_owned()
        }
        CandidateSkipReason::ApprovalRequired => "Human approval is required.".to_owned(),
        CandidateSkipReason::DependencyNotDone { dependency_id, .. } => {
            format!("Dependency {dependency_id} is not complete.")
        }
        CandidateSkipReason::AgentNotAssigned => "No Agent profile is assigned.".to_owned(),
        CandidateSkipReason::AgentCapabilityUnavailable { capability, .. } => {
            format!("Assigned Agent lacks {capability}.")
        }
        CandidateSkipReason::LowerRanked => "A higher-priority candidate was selected.".to_owned(),
    }
}

fn no_candidate_reason(reason: NoCandidateReason) -> String {
    match reason {
        NoCandidateReason::GlobalCapacityReached { .. } => {
            "Safe Autopilot paused because global capacity is full.".to_owned()
        }
        NoCandidateReason::NoRunnableCandidate => {
            "No Work item currently satisfies every safe-start rule.".to_owned()
        }
    }
}

fn autopilot_stop_reason(reason: &AutopilotStopReason) -> String {
    match reason {
        AutopilotStopReason::InvalidGlobalConcurrencyCap => {
            "Safe Autopilot stopped because its global capacity is invalid.".to_owned()
        }
        AutopilotStopReason::InvalidProjectCapacity { project_id }
        | AutopilotStopReason::DuplicateProject { project_id }
        | AutopilotStopReason::ProjectNotFound { project_id } => {
            format!("Safe Autopilot stopped on Project {project_id}.")
        }
        AutopilotStopReason::DuplicateWorkItem { work_item_id } => {
            format!("Safe Autopilot found duplicate Work item {work_item_id}.")
        }
        AutopilotStopReason::DuplicateAgentProfile { agent_profile_id } => {
            format!("Safe Autopilot found duplicate Agent profile {agent_profile_id}.")
        }
        AutopilotStopReason::DependencyNotFound {
            work_item_id,
            dependency_id,
        } => format!("{work_item_id} refers to missing dependency {dependency_id}."),
        AutopilotStopReason::AgentProfileNotFound {
            work_item_id,
            agent_profile_id,
        } => format!("{work_item_id} refers to missing Agent profile {agent_profile_id}."),
    }
}

fn join_warnings<const N: usize>(warnings: [Option<String>; N]) -> Option<String> {
    let messages = warnings.into_iter().flatten().collect::<Vec<_>>();
    (!messages.is_empty()).then(|| messages.join(" · "))
}

fn display_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_workspace_marker_requires_a_non_empty_value() {
        assert!(!demo_workspace_from_marker(None));
        assert!(!demo_workspace_from_marker(Some(OsString::new())));
        assert!(demo_workspace_from_marker(Some(OsString::from("demo"))));
    }

    #[test]
    fn desktop_state_parser_accepts_only_the_canonical_vocabulary() {
        assert_eq!(
            parse_work_item_state("in_progress"),
            Ok(WorkItemState::InProgress)
        );
        assert!(parse_work_item_state("running").is_err());
    }

    #[test]
    fn execution_workspace_view_keeps_portable_paths_out_of_the_primary_label() {
        for location in [r"C:\work\gareji-board", "/work/gareji-board"] {
            let connection = ExecutionWorkspaceConnection {
                project_id: "board".to_owned(),
                kind: ExecutionWorkspaceKind::LocalDirectory,
                location: Some(location.to_owned()),
            };
            let view = ExecutionWorkspaceView::from(&connection);
            assert_eq!(view.display_name, "gareji-board");
            assert_eq!(view.location, connection.location);
        }
    }
}

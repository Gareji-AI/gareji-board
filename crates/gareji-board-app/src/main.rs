use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use dioxus::desktop::{Config, LogicalSize, WindowBuilder};
use dioxus::prelude::*;
use gareji_board_core::{
    BlueprintApplicationBlockedReason, BlueprintApplicationPlanner, BlueprintApplicationPreview,
    BlueprintApplicationProposal, BlueprintDraft, BlueprintPlanningFacts, BoardRunController,
    ControlNodeController, ControlNodeTransitionReceipt, CoreProgressReader, EvidenceRouteRequest,
    GraphDraft, GraphRewriteController, HumanApprovalRequest, MarkdownApproachNoteReader,
    PortfolioDraft, PortfolioOrchestrationController, PortfolioPreviewBlockedReason,
    PortfolioPreviewFacts, PortfolioRunController, PortfolioScheduler, PortfolioStepOutcome,
    PortfolioStepPreview, PortfolioTickMode, PortfolioTickRequest,
};
use gareji_board_domain::{
    ActivityTimeline, AgentPlan, AgentPlanUpdateReceipt, AgentPlanUpdateRequest,
    AgentProfileSaveReceipt, AgentProfileSaveRequest, AgentProfileSummary, ApproachNoteManifest,
    ApproachRisk, ApprovalRequirement, AttachmentReceipt, AttachmentRequest, AttachmentTarget,
    AutopilotStopReason, BlueprintApplication, BlueprintApplicationReceipt,
    BlueprintApproachNotePin, BlueprintLinkKind, BlueprintNode, BlueprintNodeKind,
    BlueprintRuntimeBinding, CandidateSkipReason, CheckpointDeliveryStatus, CheckpointOutcome,
    CheckpointReconciliation, ControlGraphRevision, ControlNodeKind, ControlSignal,
    ExecutionWorkspaceConnection, ExecutionWorkspaceKind, ExecutionWorkspaceSaveReceipt,
    ExecutionWorkspaceSaveRequest, GraphCanvasLayout, GraphCanvasNodePosition,
    GraphRewriteDecision, GraphRewriteDecisionReceipt, GraphRewriteDecisionRequest,
    GraphRewriteOperation, GraphRewriteProposal, GraphRewriteProposalRequest,
    GraphRewriteProposalStatus, NoCandidateReason, NoteSocketKind, OrchestrationBlueprintRevision,
    PortfolioApprovalDecision, PortfolioNodeKind, PortfolioOrchestrationRevision,
    PortfolioPostAction, PortfolioProjectSelector, PortfolioRun, PortfolioRunStatus,
    PortfolioRunStep, PortfolioSchedule, PortfolioScheduleControl,
    PortfolioScheduleControlSaveRequest, PortfolioSignal, PortfolioSnapshot, ProgressActivity,
    ProjectCreateReceipt, ProjectCreateRequest, ProjectGraphBinding,
    ProjectGraphBindingSaveReceipt, ProjectGraphBindingSaveRequest, ProjectHealth, ProjectSummary,
    ReconciliationDecision, ReconciliationReceipt, ReconciliationRequest, RouteDecision,
    SafeAutopilotOutcome, SafeAutopilotPreview, WorkItemCreateReceipt, WorkItemCreateRequest,
    WorkItemGraphPosition, WorkItemState, WorkItemSummary, WorkItemTransitionReceipt,
    WorkItemTransitionRequest,
};
use gareji_board_store::{SqliteBoardStore, default_board_database_path};

mod agent_behavior;
mod app_navigation;
mod execution_workspace;
mod graph_layout;

use graph_layout::{CanvasPoint, ControlGraphLayout};

#[derive(Clone, Debug, PartialEq)]
enum GraphEditorAction {
    Topology,
    Layout(HashMap<String, CanvasPoint>),
    TopologyAndLayout(HashMap<String, CanvasPoint>),
}

#[derive(Clone, Debug, PartialEq)]
struct PendingControlBranchSpawn {
    source_node_id: String,
    signal: ControlSignal,
    point: CanvasPoint,
}

use app_navigation::{
    AppNavigation, AppPage, DetailDialog, LaunchCard, LaunchCardPresentation, PageIntro,
    WorkspacePage, WorkspacePageHeader, focus_page_start,
};

use agent_behavior::{
    AgentBehaviorInspection, AgentBehaviorInspector, BehaviorInspectionStatus, ReferenceInspection,
    ReferenceStatus,
};
use execution_workspace::{
    ExecutionWorkspaceConnector, ExecutionWorkspaceInspection, WorkspaceAvailability,
};

const APP_CSS: &str = include_str!("style.css");
const PREVIEW_GLOBAL_CONCURRENCY_CAP: u32 = 2;
const DESKTOP_WINDOW_WIDTH: f64 = 1440.0;
const DESKTOP_WINDOW_HEIGHT: f64 = 810.0;
const DESKTOP_MINIMUM_WIDTH: f64 = 1024.0;
const DESKTOP_MINIMUM_HEIGHT: f64 = 576.0;
const WORK_ITEM_STATES: [WorkItemState; 7] = [
    WorkItemState::Backlog,
    WorkItemState::Todo,
    WorkItemState::InProgress,
    WorkItemState::InReview,
    WorkItemState::Blocked,
    WorkItemState::Done,
    WorkItemState::Cancelled,
];

fn main() {
    let window = WindowBuilder::new()
        .with_title("Gareji Board")
        .with_inner_size(LogicalSize::new(
            DESKTOP_WINDOW_WIDTH,
            DESKTOP_WINDOW_HEIGHT,
        ))
        .with_min_inner_size(LogicalSize::new(
            DESKTOP_MINIMUM_WIDTH,
            DESKTOP_MINIMUM_HEIGHT,
        ));
    dioxus::LaunchBuilder::desktop()
        .with_cfg(Config::new().with_window(window))
        .launch(App);
}

#[derive(Clone)]
struct AppState {
    portfolio: PortfolioSnapshot,
    work_items: Vec<WorkItemSummary>,
    agent_profiles: Vec<AgentProfileSummary>,
    execution_workspaces: Vec<ExecutionWorkspaceConnection>,
    control_graphs: Vec<ControlGraphRevision>,
    graph_canvas_layouts: Vec<GraphCanvasLayout>,
    project_graph_bindings: Vec<ProjectGraphBinding>,
    graph_rewrite_proposals: Vec<GraphRewriteProposal>,
    work_item_graph_positions: Vec<WorkItemGraphPosition>,
    route_decisions: Vec<RouteDecision>,
    portfolio_orchestrations: Vec<PortfolioOrchestrationRevision>,
    portfolio_orchestration_preview: Option<PortfolioStepPreview>,
    portfolio_runs: Vec<PortfolioRun>,
    portfolio_run_steps: Vec<PortfolioRunStep>,
    portfolio_schedule_controls: Vec<PortfolioScheduleControl>,
    portfolio_orchestration_notice: Option<String>,
    orchestration_blueprints: Vec<OrchestrationBlueprintRevision>,
    approach_notes: Vec<ApproachNoteManifest>,
    approach_note_warning: Option<String>,
    blueprint_application_preview: Option<BlueprintApplicationPreview>,
    blueprint_applications: Vec<BlueprintApplicationReceipt>,
    blueprint_application_notice: Option<String>,
    blueprint_revision_notice: Option<String>,
    activity: ActivityTimeline,
    storage_label: String,
    warning: Option<String>,
    activity_warning: Option<String>,
    reconciliation_notice: Option<String>,
    attachment_notice: Option<String>,
    transition_notice: Option<String>,
    agent_plan_notice: Option<String>,
    agent_profile_notice: Option<String>,
    execution_workspace_notice: Option<String>,
    project_notice: Option<String>,
    graph_binding_notice: Option<String>,
    graph_rewrite_notice: Option<String>,
    work_item_creation_notice: Option<String>,
    run_notice: Option<String>,
    control_node_notice: Option<String>,
    run_in_progress: bool,
    autopilot_preview: Option<SafeAutopilotPreview>,
}

#[derive(Clone, Copy)]
struct NewAgentProfileSignals {
    profile_id: Signal<String>,
    role: Signal<String>,
    capabilities: Signal<String>,
    instruction_ref: Signal<String>,
    skill_refs: Signal<String>,
}

#[derive(Clone, Copy)]
struct NewProjectSignals {
    project_id: Signal<String>,
    name: Signal<String>,
    execution_cap: Signal<String>,
    workspace_location: Signal<String>,
}

#[derive(Clone, Copy)]
struct NewWorkItemSignals {
    project_id: Signal<String>,
    work_item_id: Signal<String>,
    title: Signal<String>,
    priority: Signal<String>,
}

fn use_new_agent_profile_signals() -> NewAgentProfileSignals {
    NewAgentProfileSignals {
        profile_id: use_signal(String::new),
        role: use_signal(String::new),
        capabilities: use_signal(String::new),
        instruction_ref: use_signal(String::new),
        skill_refs: use_signal(String::new),
    }
}

fn use_new_project_signals() -> NewProjectSignals {
    NewProjectSignals {
        project_id: use_signal(String::new),
        name: use_signal(String::new),
        execution_cap: use_signal(|| "1".to_owned()),
        workspace_location: use_signal(String::new),
    }
}

fn use_new_work_item_signals() -> NewWorkItemSignals {
    NewWorkItemSignals {
        project_id: use_signal(String::new),
        work_item_id: use_signal(String::new),
        title: use_signal(String::new),
        priority: use_signal(|| "100".to_owned()),
    }
}

#[allow(clippy::too_many_lines)]
fn load_app_state() -> AppState {
    let database_path = board_database_path();
    let storage_label = database_path.display().to_string();
    let (approach_notes, approach_note_warning) = load_approach_notes();
    let loaded = SqliteBoardStore::open(&database_path).and_then(|mut store| {
        store.seed_sample_if_empty()?;
        store.ensure_builtin_control_graphs()?;
        store.ensure_builtin_portfolio_orchestrations()?;
        store.ensure_builtin_orchestration_blueprints()?;
        let portfolio = store.load_portfolio()?;
        let work_items = store.load_work_items()?;
        let agent_profiles = store.load_agent_profiles()?;
        let execution_workspaces = store.load_execution_workspaces()?;
        let control_graphs = store.load_control_graph_revisions()?;
        let graph_canvas_layouts = store.load_graph_canvas_layouts()?;
        let portfolio_orchestrations = store.load_portfolio_orchestration_revisions()?;
        let mut portfolio_runs = Vec::new();
        let mut portfolio_run_steps = Vec::new();
        let mut portfolio_schedule_controls = Vec::new();
        for revision in &portfolio_orchestrations {
            portfolio_schedule_controls.push(store.load_portfolio_schedule_control(revision)?);
            if let Some(run) = store
                .load_latest_portfolio_run(&revision.orchestration_id, &revision.revision_id)?
            {
                portfolio_run_steps.extend(store.load_portfolio_run_steps(&run.run_id)?);
                portfolio_runs.push(run);
            }
        }
        let orchestration_blueprints = store.load_orchestration_blueprint_revisions()?;
        let blueprint_applications = store.load_blueprint_applications()?;
        let project_graph_bindings = store.load_project_graph_bindings()?;
        let mut graph_rewrite_proposals = Vec::new();
        for project in &portfolio.projects {
            graph_rewrite_proposals
                .extend(store.load_project_graph_rewrite_proposals(&project.id)?);
        }
        let work_item_graph_positions = store.load_work_item_graph_positions()?;
        let mut route_decisions = Vec::new();
        for project in &portfolio.projects {
            route_decisions.extend(store.load_project_route_decisions(&project.id)?);
        }
        Ok((
            store,
            portfolio,
            work_items,
            agent_profiles,
            execution_workspaces,
            control_graphs,
            graph_canvas_layouts,
            portfolio_orchestrations,
            portfolio_runs,
            portfolio_run_steps,
            portfolio_schedule_controls,
            orchestration_blueprints,
            blueprint_applications,
            project_graph_bindings,
            graph_rewrite_proposals,
            work_item_graph_positions,
            route_decisions,
        ))
    });

    match loaded {
        Ok((
            store,
            portfolio,
            work_items,
            agent_profiles,
            execution_workspaces,
            control_graphs,
            graph_canvas_layouts,
            portfolio_orchestrations,
            portfolio_runs,
            portfolio_run_steps,
            portfolio_schedule_controls,
            orchestration_blueprints,
            blueprint_applications,
            project_graph_bindings,
            graph_rewrite_proposals,
            work_item_graph_positions,
            route_decisions,
        )) => {
            let project_ids = portfolio
                .projects
                .iter()
                .map(|project| project.id.clone())
                .collect::<Vec<_>>();
            let (mut activity, mut activity_warning) = CoreProgressReader::from_environment()
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
            if let Err(error) = store.hydrate_activity(&mut activity) {
                activity_warning = Some(format!("Board activity state: {error}"));
            }
            AppState {
                portfolio,
                work_items,
                agent_profiles,
                execution_workspaces,
                control_graphs,
                graph_canvas_layouts,
                portfolio_orchestrations,
                project_graph_bindings,
                graph_rewrite_proposals,
                work_item_graph_positions,
                route_decisions,
                portfolio_orchestration_preview: None,
                portfolio_runs,
                portfolio_run_steps,
                portfolio_schedule_controls,
                portfolio_orchestration_notice: None,
                orchestration_blueprints,
                approach_notes,
                approach_note_warning,
                blueprint_application_preview: None,
                blueprint_applications,
                blueprint_application_notice: None,
                blueprint_revision_notice: None,
                activity,
                storage_label,
                warning: None,
                activity_warning,
                reconciliation_notice: None,
                attachment_notice: None,
                transition_notice: None,
                agent_plan_notice: None,
                agent_profile_notice: None,
                execution_workspace_notice: None,
                project_notice: None,
                graph_binding_notice: None,
                graph_rewrite_notice: None,
                work_item_creation_notice: None,
                run_notice: None,
                control_node_notice: None,
                run_in_progress: false,
                autopilot_preview: None,
            }
        }
        Err(error) => AppState {
            portfolio: PortfolioSnapshot::default(),
            work_items: Vec::new(),
            agent_profiles: Vec::new(),
            execution_workspaces: Vec::new(),
            control_graphs: Vec::new(),
            graph_canvas_layouts: Vec::new(),
            portfolio_orchestrations: Vec::new(),
            portfolio_orchestration_preview: None,
            portfolio_runs: Vec::new(),
            portfolio_run_steps: Vec::new(),
            portfolio_schedule_controls: Vec::new(),
            portfolio_orchestration_notice: None,
            orchestration_blueprints: Vec::new(),
            approach_notes,
            approach_note_warning,
            blueprint_application_preview: None,
            blueprint_applications: Vec::new(),
            blueprint_application_notice: None,
            blueprint_revision_notice: None,
            project_graph_bindings: Vec::new(),
            graph_rewrite_proposals: Vec::new(),
            work_item_graph_positions: Vec::new(),
            route_decisions: Vec::new(),
            activity: ActivityTimeline::default(),
            storage_label,
            warning: Some(error.to_string()),
            activity_warning: None,
            reconciliation_notice: None,
            attachment_notice: None,
            transition_notice: None,
            agent_plan_notice: None,
            agent_profile_notice: None,
            execution_workspace_notice: None,
            project_notice: None,
            graph_binding_notice: None,
            graph_rewrite_notice: None,
            work_item_creation_notice: None,
            run_notice: None,
            control_node_notice: None,
            run_in_progress: false,
            autopilot_preview: None,
        },
    }
}

fn board_database_path() -> PathBuf {
    env::var_os("GAREJI_BOARD_DB")
        .filter(|value| !value.is_empty())
        .map_or_else(default_board_database_path, PathBuf::from)
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
    let warning = if errors.is_empty() {
        None
    } else {
        Some(format!(
            "{} Approach Note(s) could not be loaded: {}",
            errors.len(),
            errors.join(" · ")
        ))
    };
    (notes, warning)
}

fn discover_default_approach_notes(root: &Path) -> Vec<PathBuf> {
    let directory = root.join("approaches");
    let Ok(entries) = fs::read_dir(directory) else {
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

fn reconcile(request: &ReconciliationRequest) -> Result<ReconciliationReceipt, String> {
    SqliteBoardStore::open(board_database_path())
        .and_then(|mut store| store.reconcile_checkpoint(request))
        .map_err(|error| error.to_string())
}

fn attach(request: &AttachmentRequest) -> Result<AttachmentReceipt, String> {
    SqliteBoardStore::open(board_database_path())
        .and_then(|mut store| store.attach_checkpoint(request))
        .map_err(|error| error.to_string())
}

fn transition_work_item(
    request: &WorkItemTransitionRequest,
) -> Result<WorkItemTransitionReceipt, String> {
    SqliteBoardStore::open(board_database_path())
        .and_then(|mut store| store.transition_work_item(request))
        .map_err(|error| error.to_string())
}

fn update_agent_plan(request: &AgentPlanUpdateRequest) -> Result<AgentPlanUpdateReceipt, String> {
    SqliteBoardStore::open(board_database_path())
        .and_then(|mut store| store.update_agent_plan(request))
        .map_err(|error| error.to_string())
}

fn save_agent_profile(
    request: &AgentProfileSaveRequest,
) -> Result<AgentProfileSaveReceipt, String> {
    SqliteBoardStore::open(board_database_path())
        .and_then(|mut store| store.save_agent_profile(request))
        .map_err(|error| error.to_string())
}

fn save_execution_workspace(
    request: &ExecutionWorkspaceSaveRequest,
) -> Result<ExecutionWorkspaceSaveReceipt, String> {
    SqliteBoardStore::open(board_database_path())
        .and_then(|mut store| store.save_execution_workspace(request))
        .map_err(|error| error.to_string())
}

fn create_project(request: &ProjectCreateRequest) -> Result<ProjectCreateReceipt, String> {
    SqliteBoardStore::open(board_database_path())
        .and_then(|mut store| store.create_project(request))
        .map_err(|error| error.to_string())
}

fn save_project_graph_binding(
    request: &ProjectGraphBindingSaveRequest,
) -> Result<ProjectGraphBindingSaveReceipt, String> {
    SqliteBoardStore::open(board_database_path())
        .and_then(|mut store| store.save_project_graph_binding(request))
        .map_err(|error| error.to_string())
}

fn propose_graph_rewrite(
    request: &GraphRewriteProposalRequest,
) -> Result<GraphRewriteProposal, String> {
    SqliteBoardStore::open(board_database_path())
        .map_err(|error| error.to_string())
        .and_then(|mut store| {
            GraphRewriteController::propose(&mut store, request).map_err(|error| error.to_string())
        })
}

fn decide_graph_rewrite(
    request: &GraphRewriteDecisionRequest,
) -> Result<GraphRewriteDecisionReceipt, String> {
    SqliteBoardStore::open(board_database_path())
        .map_err(|error| error.to_string())
        .and_then(|mut store| {
            GraphRewriteController::decide(&mut store, request).map_err(|error| error.to_string())
        })
}

fn create_work_item(request: &WorkItemCreateRequest) -> Result<WorkItemCreateReceipt, String> {
    SqliteBoardStore::open(board_database_path())
        .and_then(|mut store| store.create_work_item(request))
        .map_err(|error| error.to_string())
}

fn execute_current_agent_loop(work_item_id: &str) -> Result<String, String> {
    let mut store =
        SqliteBoardStore::open(board_database_path()).map_err(|error| error.to_string())?;
    let mut controller =
        BoardRunController::from_environment().map_err(|error| error.to_string())?;
    controller
        .run_current_agent_loop_with_defaults(&mut store, work_item_id)
        .map(|outcome| outcome.status_message())
        .map_err(|error| error.to_string())
}

fn record_evidence_route(
    request: &EvidenceRouteRequest,
) -> Result<ControlNodeTransitionReceipt, String> {
    SqliteBoardStore::open(board_database_path())
        .map_err(|error| error.to_string())
        .and_then(|mut store| {
            ControlNodeController::record_evidence_route(&mut store, request)
                .map_err(|error| error.to_string())
        })
}

fn record_human_approval(
    request: &HumanApprovalRequest,
) -> Result<ControlNodeTransitionReceipt, String> {
    SqliteBoardStore::open(board_database_path())
        .map_err(|error| error.to_string())
        .and_then(|mut store| {
            ControlNodeController::record_human_approval(&mut store, request)
                .map_err(|error| error.to_string())
        })
}

fn handle_attachment(mut state: Signal<AppState>, request: &AttachmentRequest) {
    match attach(request) {
        Ok(receipt) => {
            let mut reloaded = load_app_state();
            reloaded.attachment_notice = Some(attachment_message(&receipt));
            state.set(reloaded);
        }
        Err(error) => state.write().attachment_notice = Some(error),
    }
}

fn handle_reconciliation(mut state: Signal<AppState>, request: &ReconciliationRequest) {
    match reconcile(request) {
        Ok(receipt) => {
            let mut reloaded = load_app_state();
            reloaded.reconciliation_notice = Some(reconciliation_message(&receipt));
            state.set(reloaded);
        }
        Err(error) => state.write().reconciliation_notice = Some(error),
    }
}

fn handle_transition(mut state: Signal<AppState>, request: &WorkItemTransitionRequest) {
    match transition_work_item(request) {
        Ok(receipt) => {
            let mut reloaded = load_app_state();
            reloaded.transition_notice = Some(transition_message(&receipt));
            state.set(reloaded);
        }
        Err(error) => state.write().transition_notice = Some(error),
    }
}

fn handle_agent_plan(mut state: Signal<AppState>, request: &AgentPlanUpdateRequest) {
    match update_agent_plan(request) {
        Ok(receipt) => {
            let mut reloaded = load_app_state();
            reloaded.agent_plan_notice = Some(agent_plan_message(&receipt));
            state.set(reloaded);
        }
        Err(error) => state.write().agent_plan_notice = Some(error),
    }
}

fn handle_agent_profile(
    mut state: Signal<AppState>,
    mut form: NewAgentProfileSignals,
    request: &AgentProfileSaveRequest,
) {
    match save_agent_profile(request) {
        Ok(receipt) => {
            if receipt.previous.is_none() {
                form.profile_id.set(String::new());
                form.role.set(String::new());
                form.capabilities.set(String::new());
                form.instruction_ref.set(String::new());
                form.skill_refs.set(String::new());
            }
            let mut reloaded = load_app_state();
            reloaded.agent_profile_notice = Some(agent_profile_message(&receipt));
            state.set(reloaded);
        }
        Err(error) => state.write().agent_profile_notice = Some(error),
    }
}

fn handle_execution_workspace(
    mut state: Signal<AppState>,
    request: &ExecutionWorkspaceSaveRequest,
) {
    match save_execution_workspace(request) {
        Ok(receipt) => {
            let mut reloaded = load_app_state();
            reloaded.execution_workspace_notice = Some(execution_workspace_message(&receipt));
            state.set(reloaded);
        }
        Err(error) => state.write().execution_workspace_notice = Some(error),
    }
}

fn handle_execution_workspace_connection(
    mut state: Signal<AppState>,
    request: (String, Option<ExecutionWorkspaceConnection>, String),
) {
    let (project_id, expected, location) = request;
    match ExecutionWorkspaceConnector::connect_local_directory(&project_id, &location) {
        Ok(target) => {
            handle_execution_workspace(state, &ExecutionWorkspaceSaveRequest { expected, target });
        }
        Err(error) => state.write().execution_workspace_notice = Some(error),
    }
}

fn handle_project_creation(
    mut state: Signal<AppState>,
    mut form: NewProjectSignals,
    request: (String, String, u32, String),
) {
    let (project_id, name, execution_cap, location) = request;
    match ExecutionWorkspaceConnector::connect_local_directory(&project_id, &location) {
        Ok(execution_workspace) => match create_project(&ProjectCreateRequest {
            id: project_id,
            name,
            execution_cap,
            execution_workspace,
        }) {
            Ok(receipt) => {
                form.project_id.set(String::new());
                form.name.set(String::new());
                form.execution_cap.set("1".to_owned());
                form.workspace_location.set(String::new());
                let mut reloaded = load_app_state();
                reloaded.project_notice = Some(project_message(&receipt));
                state.set(reloaded);
            }
            Err(error) => state.write().project_notice = Some(error),
        },
        Err(error) => state.write().project_notice = Some(error),
    }
}

fn handle_project_graph_binding(
    mut state: Signal<AppState>,
    request: &ProjectGraphBindingSaveRequest,
) {
    match save_project_graph_binding(request) {
        Ok(receipt) => {
            let mut reloaded = load_app_state();
            reloaded.graph_binding_notice = Some(format!(
                "{} now uses {} / {} from entry {}.",
                receipt.resulting.project_id,
                receipt.resulting.graph_id,
                receipt.resulting.revision_id,
                receipt.resulting.entry_id
            ));
            state.set(reloaded);
        }
        Err(error) => state.write().graph_binding_notice = Some(error),
    }
}

fn handle_graph_canvas_layout(mut state: Signal<AppState>, layout: GraphCanvasLayout) {
    let result = SqliteBoardStore::open(board_database_path())
        .and_then(|mut store| store.save_graph_canvas_layout(&layout));
    match result {
        Ok(()) => {
            let mut snapshot = state.write();
            if let Some(existing) = snapshot
                .graph_canvas_layouts
                .iter_mut()
                .find(|existing| existing.graph_id == layout.graph_id)
            {
                *existing = layout;
            } else {
                snapshot.graph_canvas_layouts.push(layout);
            }
        }
        Err(error) => {
            state.write().graph_rewrite_notice =
                Some(format!("Graph layout was not saved: {error}"));
        }
    }
}

fn handle_graph_rewrite_proposal(
    mut state: Signal<AppState>,
    request: &GraphRewriteProposalRequest,
) {
    match propose_graph_rewrite(request) {
        Ok(proposal) => {
            let mut reloaded = load_app_state();
            reloaded.graph_rewrite_notice = Some(format!(
                "Graph change {} is waiting for approval; the active revision is unchanged.",
                proposal.proposal_id
            ));
            state.set(reloaded);
        }
        Err(error) => state.write().graph_rewrite_notice = Some(error),
    }
}

fn handle_graph_rewrite_decision(
    mut state: Signal<AppState>,
    request: &GraphRewriteDecisionRequest,
) {
    match decide_graph_rewrite(request) {
        Ok(receipt) => {
            let mut reloaded = load_app_state();
            reloaded.graph_rewrite_notice = Some(if receipt.published {
                format!(
                    "Approved {}. Future work now uses revision {}; active work stays pinned.",
                    receipt.proposal.proposal_id, receipt.resulting_binding.revision_id
                )
            } else {
                format!(
                    "Rejected {}. No Graph revision or Project setting changed.",
                    receipt.proposal.proposal_id
                )
            });
            state.set(reloaded);
        }
        Err(error) => state.write().graph_rewrite_notice = Some(error),
    }
}

fn handle_work_item_creation(
    mut state: Signal<AppState>,
    mut form: NewWorkItemSignals,
    request: &WorkItemCreateRequest,
) {
    match create_work_item(request) {
        Ok(receipt) => {
            form.work_item_id.set(String::new());
            form.title.set(String::new());
            form.priority.set("100".to_owned());
            let mut reloaded = load_app_state();
            reloaded.work_item_creation_notice = Some(work_item_creation_message(&receipt));
            state.set(reloaded);
        }
        Err(error) => state.write().work_item_creation_notice = Some(error),
    }
}

fn handle_board_run(mut state: Signal<AppState>, work_item_id: String) {
    if state.read().run_in_progress {
        return;
    }
    {
        let mut current = state.write();
        current.run_in_progress = true;
        current.run_notice = Some(format!(
            "Starting the current Agent Loop for {work_item_id}. The isolated Run continues in the background."
        ));
    }
    spawn(async move {
        let result = tokio::task::spawn_blocking(move || execute_current_agent_loop(&work_item_id))
            .await
            .map_err(|error| format!("The background Run task stopped unexpectedly: {error}"))
            .and_then(std::convert::identity);
        let mut reloaded = load_app_state();
        reloaded.run_notice = Some(result.unwrap_or_else(|error| error));
        reloaded.run_in_progress = false;
        state.set(reloaded);
    });
}

fn handle_evidence_route(mut state: Signal<AppState>, request: &EvidenceRouteRequest) {
    match record_evidence_route(request) {
        Ok(receipt) => {
            let mut reloaded = load_app_state();
            reloaded.control_node_notice = Some(control_node_transition_message(&receipt));
            state.set(reloaded);
        }
        Err(error) => state.write().control_node_notice = Some(error),
    }
}

fn handle_human_approval(mut state: Signal<AppState>, request: &HumanApprovalRequest) {
    match record_human_approval(request) {
        Ok(receipt) => {
            let mut reloaded = load_app_state();
            reloaded.control_node_notice = Some(control_node_transition_message(&receipt));
            state.set(reloaded);
        }
        Err(error) => state.write().control_node_notice = Some(error),
    }
}

fn handle_portfolio_orchestration_preview(
    mut state: Signal<AppState>,
    orchestration_id: &str,
    revision_id: &str,
) {
    let preview = {
        let snapshot = state.read();
        snapshot
            .portfolio_orchestrations
            .iter()
            .find(|revision| {
                revision.orchestration_id == orchestration_id && revision.revision_id == revision_id
            })
            .map(|revision| {
                let facts = PortfolioPreviewFacts {
                    portfolio: &snapshot.portfolio,
                    work_items: &snapshot.work_items,
                    agent_profiles: &snapshot.agent_profiles,
                    project_graph_bindings: &snapshot.project_graph_bindings,
                    control_graphs: &snapshot.control_graphs,
                };
                snapshot
                    .portfolio_runs
                    .iter()
                    .find(|run| {
                        run.orchestration_id == orchestration_id
                            && run.revision_id == revision_id
                            && run.status == PortfolioRunStatus::Active
                    })
                    .map_or_else(
                        || PortfolioOrchestrationController::preview_next(revision, &facts),
                        |run| {
                            PortfolioOrchestrationController::preview_node(
                                revision,
                                &run.current_node_id,
                                &facts,
                            )
                        },
                    )
            })
    };
    state.write().portfolio_orchestration_preview = preview;
}

fn handle_portfolio_tick(mut state: Signal<AppState>, orchestration_id: &str, revision_id: &str) {
    let snapshot = state.read().clone();
    match perform_portfolio_tick(
        &snapshot,
        orchestration_id,
        revision_id,
        PortfolioTickMode::Force,
    ) {
        Ok(Some(message)) => {
            let mut reloaded = load_app_state();
            reloaded.portfolio_orchestration_notice = Some(message);
            state.set(reloaded);
        }
        Ok(None) => {
            state.write().portfolio_orchestration_notice =
                Some("No Portfolio node was ready to run.".to_owned());
        }
        Err(error) => state.write().portfolio_orchestration_notice = Some(error),
    }
}

fn handle_portfolio_schedule_control(
    mut state: Signal<AppState>,
    orchestration_id: &str,
    revision_id: &str,
    automatic_ticks_enabled: bool,
) {
    let expected = state
        .read()
        .portfolio_schedule_controls
        .iter()
        .find(|control| {
            control.orchestration_id == orchestration_id && control.revision_id == revision_id
        })
        .cloned();
    let Some(expected) = expected else {
        state.write().portfolio_orchestration_notice =
            Some("Portfolio schedule control was not found.".to_owned());
        return;
    };
    let request = PortfolioScheduleControlSaveRequest {
        target: PortfolioScheduleControl {
            orchestration_id: orchestration_id.to_owned(),
            revision_id: revision_id.to_owned(),
            automatic_ticks_enabled,
        },
        expected,
    };
    let result = SqliteBoardStore::open(board_database_path())
        .and_then(|mut store| store.save_portfolio_schedule_control(&request));
    match result {
        Ok(receipt) => {
            let mut reloaded = load_app_state();
            reloaded.portfolio_orchestration_notice =
                Some(if receipt.resulting.automatic_ticks_enabled {
                    "Automatic Portfolio ticks resumed. The current Run position was preserved."
                        .to_owned()
                } else {
                    "Automatic Portfolio ticks paused. Manual one-node runs remain available."
                        .to_owned()
                });
            state.set(reloaded);
        }
        Err(error) => state.write().portfolio_orchestration_notice = Some(error.to_string()),
    }
}

fn handle_portfolio_revision_save(
    mut state: Signal<AppState>,
    revision: &PortfolioOrchestrationRevision,
) {
    let identity = format!("{}/{}", revision.orchestration_id, revision.revision_id);
    let result = SqliteBoardStore::open(board_database_path())
        .and_then(|mut store| store.save_portfolio_orchestration_revision(revision));
    match result {
        Ok(inserted) => {
            let mut reloaded = load_app_state();
            reloaded.portfolio_orchestration_notice = Some(if inserted {
                format!("Portfolio revision {identity} was saved as an immutable candidate.")
            } else {
                format!("Portfolio revision {identity} was already stored unchanged.")
            });
            state.set(reloaded);
        }
        Err(error) => {
            state.write().portfolio_orchestration_notice =
                Some(format!("Portfolio revision was not saved: {error}"));
        }
    }
}

fn handle_portfolio_approval(
    mut state: Signal<AppState>,
    orchestration_id: &str,
    revision_id: &str,
    decision: PortfolioApprovalDecision,
) {
    let snapshot = state.read().clone();
    let result = (|| {
        let revision = snapshot
            .portfolio_orchestrations
            .iter()
            .find(|revision| {
                revision.orchestration_id == orchestration_id && revision.revision_id == revision_id
            })
            .ok_or_else(|| "Portfolio orchestration revision was not found.".to_owned())?;
        let now_epoch_seconds = current_epoch_seconds()?;
        let mut store =
            SqliteBoardStore::open(board_database_path()).map_err(|error| error.to_string())?;
        let current_run = store
            .load_latest_portfolio_run(orchestration_id, revision_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "Portfolio Run was not found.".to_owned())?;
        let receipt = PortfolioRunController::resolve_approval(
            revision,
            &current_run,
            decision,
            now_epoch_seconds,
        )
        .map_err(|error| error.to_string())?;
        store
            .record_portfolio_tick(Some(&current_run), &receipt.resulting, &receipt.step)
            .map_err(|error| error.to_string())?;
        Ok::<_, String>(format!(
            "Portfolio approval recorded as {}. Run advanced to {}.",
            portfolio_signal_label(decision.signal()),
            receipt.resulting.current_node_id
        ))
    })();
    match result {
        Ok(message) => {
            let mut reloaded = load_app_state();
            reloaded.portfolio_orchestration_notice = Some(message);
            state.set(reloaded);
        }
        Err(error) => state.write().portfolio_orchestration_notice = Some(error),
    }
}

fn handle_due_portfolio_ticks(mut state: Signal<AppState>) {
    let result = current_epoch_seconds().and_then(|now| {
        PortfolioScheduler::open(board_database_path())
            .map_err(|error| error.to_string())
            .and_then(|mut scheduler| {
                scheduler
                    .tick_due_once(now)
                    .map_err(|error| error.to_string())
            })
    });
    match result {
        Ok(report) if !report.recorded_ticks.is_empty() || report.contended_revisions > 0 => {
            let mut reloaded = load_app_state();
            reloaded.portfolio_orchestration_notice = Some(format!(
                "Automatic Portfolio pass recorded {} tick(s); {} revision(s) changed concurrently.",
                report.recorded_ticks.len(),
                report.contended_revisions
            ));
            state.set(reloaded);
        }
        Ok(_) => {}
        Err(error) => {
            state.write().portfolio_orchestration_notice =
                Some(format!("Automatic Portfolio pass failed: {error}"));
        }
    }
}

fn perform_portfolio_tick(
    snapshot: &AppState,
    orchestration_id: &str,
    revision_id: &str,
    mode: PortfolioTickMode,
) -> Result<Option<String>, String> {
    let revision = snapshot
        .portfolio_orchestrations
        .iter()
        .find(|revision| {
            revision.orchestration_id == orchestration_id && revision.revision_id == revision_id
        })
        .ok_or_else(|| "Portfolio orchestration revision was not found.".to_owned())?;
    let now_epoch_seconds = current_epoch_seconds()?;
    let mut store =
        SqliteBoardStore::open(board_database_path()).map_err(|error| error.to_string())?;
    let latest = store
        .load_latest_portfolio_run(orchestration_id, revision_id)
        .map_err(|error| error.to_string())?;
    let current_run = match latest.as_ref() {
        Some(run) if run.status == PortfolioRunStatus::Completed => {
            if mode == PortfolioTickMode::DueOnly
                && run
                    .next_tick_at_epoch_seconds
                    .is_none_or(|due| due > now_epoch_seconds)
            {
                return Ok(None);
            }
            None
        }
        Some(run) if mode == PortfolioTickMode::DueOnly && !run.is_due(now_epoch_seconds) => {
            return Ok(None);
        }
        Some(run) => Some(run),
        None if mode == PortfolioTickMode::DueOnly
            && revision.schedule.interval_seconds().is_none() =>
        {
            return Ok(None);
        }
        None => None,
    };
    let run_id = next_portfolio_run_id();
    let facts = PortfolioPreviewFacts {
        portfolio: &snapshot.portfolio,
        work_items: &snapshot.work_items,
        agent_profiles: &snapshot.agent_profiles,
        project_graph_bindings: &snapshot.project_graph_bindings,
        control_graphs: &snapshot.control_graphs,
    };
    let receipt = PortfolioRunController::tick(&PortfolioTickRequest {
        revision,
        current_run,
        run_id: &run_id,
        now_epoch_seconds,
        mode,
        facts: &facts,
    })
    .map_err(|error| error.to_string())?;
    store
        .record_portfolio_tick(latest.as_ref(), &receipt.resulting, &receipt.step)
        .map_err(|error| error.to_string())?;
    Ok(Some(format!(
        "Portfolio tick recorded node {} (Run {}, step {}).",
        receipt.step.node_id, receipt.resulting.run_id, receipt.step.sequence
    )))
}

fn current_epoch_seconds() -> Result<i64, String> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("System clock is before Unix epoch: {error}"))?
        .as_secs();
    i64::try_from(seconds).map_err(|_| "System clock is outside the supported range.".to_owned())
}

fn next_portfolio_run_id() -> String {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("portfolio-run-{}", elapsed.as_nanos())
}

fn handle_blueprint_application_preview(mut state: Signal<AppState>) {
    let preview = {
        let snapshot = state.read();
        BlueprintApplicationPlanner::preview(&BlueprintPlanningFacts {
            blueprints: &snapshot.orchestration_blueprints,
            approach_notes: &snapshot.approach_notes,
            portfolio: &snapshot.portfolio,
            work_items: &snapshot.work_items,
            agent_profiles: &snapshot.agent_profiles,
            execution_workspaces: &snapshot.execution_workspaces,
        })
    };
    state.write().blueprint_application_preview = Some(preview);
}

fn handle_blueprint_application_accept(
    mut state: Signal<AppState>,
    proposal: BlueprintApplicationProposal,
) {
    let application_id = next_blueprint_application_id();
    let application = BlueprintApplication {
        application_id: application_id.clone(),
        blueprint_id: proposal.blueprint_id,
        revision_id: proposal.revision_id,
        entry_node_id: proposal.entry_node_id,
        project_id: proposal.project_id.clone(),
        work_item_id: proposal.work_item.id.clone(),
    };
    let runtime_binding = BlueprintRuntimeBinding {
        application_id,
        project_id: proposal.project_id,
        work_item_id: proposal.work_item.id,
        agent_profile_id: proposal.agent_profile_id,
        execution_workspace: proposal.execution_workspace,
        approach_notes: proposal
            .approach_notes
            .into_iter()
            .map(|note| BlueprintApproachNotePin {
                approach_id: note.approach_id,
                absolute_path: note.absolute_path,
                fingerprint: note.fingerprint,
            })
            .collect(),
    };
    let result = SqliteBoardStore::open(board_database_path())
        .and_then(|mut store| store.accept_blueprint_application(&application, &runtime_binding));
    match result {
        Ok(receipt) => {
            let message = format!(
                "Blueprint applied to {}. Runtime facts are pinned; no Runner was started.",
                receipt.application.work_item_id
            );
            let mut snapshot = state.write();
            snapshot.blueprint_applications.push(receipt);
            snapshot.blueprint_application_notice = Some(message);
        }
        Err(error) => {
            state.write().blueprint_application_notice =
                Some(format!("Blueprint application was not recorded: {error}"));
        }
    }
}

fn handle_blueprint_revision_save(
    mut state: Signal<AppState>,
    revision: OrchestrationBlueprintRevision,
) {
    let result = SqliteBoardStore::open(board_database_path())
        .and_then(|mut store| store.save_orchestration_blueprint_revision(&revision));
    match result {
        Ok(inserted) => {
            let identity = format!("{} · {}", revision.blueprint_id, revision.revision_id);
            if inserted {
                state.write().orchestration_blueprints.push(revision);
                state
                    .write()
                    .orchestration_blueprints
                    .sort_by(|left, right| {
                        left.blueprint_id
                            .cmp(&right.blueprint_id)
                            .then_with(|| left.revision_id.cmp(&right.revision_id))
                    });
                state.write().blueprint_revision_notice = Some(format!(
                    "Saved immutable Blueprint revision {identity}. No Project or Runner was changed."
                ));
            } else {
                state.write().blueprint_revision_notice =
                    Some(format!("Blueprint revision {identity} was already saved."));
            }
        }
        Err(error) => {
            state.write().blueprint_revision_notice =
                Some(format!("Blueprint revision was not saved: {error}"));
        }
    }
}

#[allow(clippy::too_many_lines, non_snake_case)]
fn App() -> Element {
    let mut state = use_signal(load_app_state);
    let mut active_page = use_signal(|| AppPage::Overview);
    let mut active_workspace = use_signal(|| None::<WorkspacePage>);
    use_future(move || async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            handle_due_portfolio_ticks(state);
        }
    });
    let new_profile_form = use_new_agent_profile_signals();
    let new_project_form = use_new_project_signals();
    let new_work_item_form = use_new_work_item_signals();
    let snapshot = state.read().clone();
    let project_count = snapshot.portfolio.projects.len();
    let active_runs = snapshot.portfolio.active_runs();
    let blocked_items = snapshot.portfolio.blocked_items();
    let delivery_issues = snapshot.activity.delivery_issues();
    let preview_portfolio = snapshot.portfolio.clone();
    let preview_work_items = snapshot.work_items.clone();
    let preview_agent_profiles = snapshot.agent_profiles.clone();
    let on_preview = move |_| {
        state.write().autopilot_preview = Some(SafeAutopilotPreview::evaluate(
            &preview_portfolio,
            &preview_work_items,
            &preview_agent_profiles,
            PREVIEW_GLOBAL_CONCURRENCY_CAP,
        ));
    };
    let on_attach = move |request| handle_attachment(state, &request);
    let on_reconcile = move |request| handle_reconciliation(state, &request);
    let on_transition = move |request| handle_transition(state, &request);
    let on_agent_plan = move |request| handle_agent_plan(state, &request);
    let on_agent_profile = move |request| handle_agent_profile(state, new_profile_form, &request);
    let on_execution_workspace = move |(project_id, expected, location)| {
        handle_execution_workspace_connection(state, (project_id, expected, location));
    };
    let on_project_create = move |request| {
        handle_project_creation(state, new_project_form, request);
    };
    let on_graph_select = move |request| handle_project_graph_binding(state, &request);
    let on_graph_canvas_layout = move |layout| handle_graph_canvas_layout(state, layout);
    let on_graph_rewrite_propose = move |request| handle_graph_rewrite_proposal(state, &request);
    let on_graph_rewrite_decide = move |request| handle_graph_rewrite_decision(state, &request);
    let on_work_item_create = move |request| {
        handle_work_item_creation(state, new_work_item_form, &request);
    };
    let on_run = move |work_item_id| handle_board_run(state, work_item_id);
    let on_evidence_route = move |request| handle_evidence_route(state, &request);
    let on_human_approval = move |request| handle_human_approval(state, &request);
    let on_portfolio_orchestration_preview =
        move |(orchestration_id, revision_id): (String, String)| {
            handle_portfolio_orchestration_preview(state, &orchestration_id, &revision_id);
        };
    let on_portfolio_tick = move |(orchestration_id, revision_id): (String, String)| {
        handle_portfolio_tick(state, &orchestration_id, &revision_id);
    };
    let on_portfolio_schedule_control =
        move |(orchestration_id, revision_id, enabled): (String, String, bool)| {
            handle_portfolio_schedule_control(state, &orchestration_id, &revision_id, enabled);
        };
    let on_portfolio_approval = move |(orchestration_id, revision_id, decision): (
        String,
        String,
        PortfolioApprovalDecision,
    )| {
        handle_portfolio_approval(state, &orchestration_id, &revision_id, decision);
    };
    let on_portfolio_revision_save =
        move |revision| handle_portfolio_revision_save(state, &revision);
    let on_blueprint_application_preview = move |_| handle_blueprint_application_preview(state);
    let on_blueprint_application_accept =
        move |proposal| handle_blueprint_application_accept(state, proposal);
    let on_blueprint_revision_save =
        move |revision| handle_blueprint_revision_save(state, revision);
    let action_notices = action_notices(&snapshot);
    let active_page_value = *active_page.read();
    let active_workspace_value = *active_workspace.read();
    let mut select_page = move |page| {
        active_workspace.set(None);
        active_page.set(page);
        focus_page_start();
    };

    rsx! {
        document::Title { "Gareji Board" }
        style { {APP_CSS} }
        a { class: "skip-link", href: "#main-content", "Skip to main content" }
        main { id: "main-content", class: "shell", tabindex: "-1",
            header { class: "topbar",
                div { class: "brand-lockup",
                    div { class: "brand-mark", "G" }
                    div {
                        p { class: "eyebrow", "LOCAL AUTOPILOT CONTROL" }
                        h1 { "Gareji Board" }
                    }
                }
                div { class: "status-pill", span { class: "status-dot" } "Local · Ready" }
            }

            AppNavigation {
                active: active_page_value,
                on_select: select_page,
            }

            if let Some(warning) = &snapshot.warning {
                aside { class: "warning", "Local data could not be loaded: {warning}" }
            }

            ActionNotices { notices: action_notices }

            if let Some(workspace) = active_workspace_value {
                section {
                    class: "page-stage workspace-page",
                    aria_label: workspace.title(),
                    tabindex: "-1",
                    WorkspacePageHeader {
                        workspace,
                        return_page: active_page_value,
                        on_back: move |()| {
                            active_workspace.set(None);
                            focus_page_start();
                        },
                    }
                    div { class: "workspace-page-body",
                        {match workspace {
                            WorkspacePage::AgentProfiles => rsx! {
                                AgentProfileCatalog {
                                    agent_profiles: snapshot.agent_profiles.clone(),
                                    work_items: snapshot.work_items.clone(),
                                    portfolio: snapshot.portfolio.clone(),
                                    execution_workspaces: snapshot.execution_workspaces.clone(),
                                    new_profile_id: new_profile_form.profile_id,
                                    new_profile_role: new_profile_form.role,
                                    new_profile_capabilities: new_profile_form.capabilities,
                                    new_profile_instruction_ref: new_profile_form.instruction_ref,
                                    new_profile_skill_refs: new_profile_form.skill_refs,
                                    on_save: on_agent_profile,
                                }
                            },
                            WorkspacePage::ProjectGraphs => rsx! {
                                PortfolioGraphManager {
                                    portfolio: snapshot.portfolio.clone(),
                                    control_graphs: snapshot.control_graphs.clone(),
                                    graph_canvas_layouts: snapshot.graph_canvas_layouts.clone(),
                                    agent_profiles: snapshot.agent_profiles.clone(),
                                    project_graph_bindings: snapshot.project_graph_bindings.clone(),
                                    graph_rewrite_proposals: snapshot.graph_rewrite_proposals.clone(),
                                    work_item_graph_positions: snapshot.work_item_graph_positions.clone(),
                                    route_decisions: snapshot.route_decisions.clone(),
                                    on_graph_select,
                                    on_graph_canvas_layout,
                                    on_graph_rewrite_propose,
                                    on_graph_rewrite_decide,
                                    on_evidence_route,
                                    on_human_approval,
                                }
                            },
                            WorkspacePage::BlueprintStudio => rsx! {
                                BlueprintStudioPanel {
                                    blueprints: snapshot.orchestration_blueprints.clone(),
                                    graph_canvas_layouts: snapshot.graph_canvas_layouts.clone(),
                                    approach_notes: snapshot.approach_notes.clone(),
                                    note_warning: snapshot.approach_note_warning.clone(),
                                    preview: snapshot.blueprint_application_preview.clone(),
                                    applications: snapshot.blueprint_applications.clone(),
                                    application_notice: snapshot.blueprint_application_notice.clone(),
                                    revision_notice: snapshot.blueprint_revision_notice.clone(),
                                    on_preview: on_blueprint_application_preview,
                                    on_accept: on_blueprint_application_accept,
                                    on_save_revision: on_blueprint_revision_save,
                                    on_graph_canvas_layout,
                                }
                            },
                            WorkspacePage::PortfolioOrchestration => rsx! {
                                PortfolioOrchestrationPanel {
                                    revisions: snapshot.portfolio_orchestrations.clone(),
                                    graph_canvas_layouts: snapshot.graph_canvas_layouts.clone(),
                                    preview: snapshot.portfolio_orchestration_preview.clone(),
                                    runs: snapshot.portfolio_runs.clone(),
                                    steps: snapshot.portfolio_run_steps.clone(),
                                    schedule_controls: snapshot.portfolio_schedule_controls.clone(),
                                    projects: snapshot.portfolio.projects.clone(),
                                    notice: snapshot.portfolio_orchestration_notice.clone(),
                                    on_preview: on_portfolio_orchestration_preview,
                                    on_tick: on_portfolio_tick,
                                    on_schedule_control: on_portfolio_schedule_control,
                                    on_approval: on_portfolio_approval,
                                    on_save_revision: on_portfolio_revision_save,
                                    on_graph_canvas_layout,
                                }
                            },
                        }}
                    }
                }
            }

            if active_workspace_value.is_none() {
                section {
                class: "page-stage",
                aria_label: active_page_value.label(),
                tabindex: "-1",
                if active_page_value != AppPage::Overview {
                    PageIntro { page: active_page_value }
                }

                {match active_page_value {
                    AppPage::Overview => rsx! {
                        PortfolioHero { on_preview }
                        section { class: "metrics", aria_label: "Portfolio metrics",
                            Metric { value: project_count.to_string(), label: "Projects" }
                            Metric { value: active_runs.to_string(), label: "Active runs" }
                            Metric { value: blocked_items.to_string(), label: "Need attention" }
                            Metric { value: delivery_issues.to_string(), label: "Delivery issues" }
                        }
                        if let Some(preview) = &snapshot.autopilot_preview {
                            AutopilotPreviewPanel {
                                preview: preview.clone(),
                                run_in_progress: snapshot.run_in_progress,
                                graph_positions: snapshot.work_item_graph_positions.clone(),
                                control_graphs: snapshot.control_graphs.clone(),
                                on_run,
                                on_evidence_route,
                                on_human_approval,
                            }
                        }
                        section { class: "overview-links", aria_label: "Explore Gareji Board",
                            LaunchCard {
                                presentation: LaunchCardPresentation::Page,
                                kicker: "Plan",
                                title: "Work and agents",
                                description: "Open the Kanban first. Agent profiles stay one click away.",
                                meta: format!("{} work items", snapshot.work_items.len()),
                                on_open: move |_| select_page(AppPage::Work),
                            }
                            LaunchCard {
                                presentation: LaunchCardPresentation::Page,
                                kicker: "Connect",
                                title: "Managed projects",
                                description: "Review project health and local workspace connections.",
                                meta: format!("{} projects", snapshot.portfolio.projects.len()),
                                on_open: move |_| select_page(AppPage::Projects),
                            }
                            LaunchCard {
                                presentation: LaunchCardPresentation::Page,
                                kicker: "Observe",
                                title: "Activity and evidence",
                                description: "Handle checkpoint links, delivery issues, and reconciliation.",
                                meta: format!("{} delivery issues", delivery_issues),
                                on_open: move |_| select_page(AppPage::Activity),
                            }
                        }
                    },
                    AppPage::Work => rsx! {
                        section { class: "page-toolbar",
                            div {
                                strong { "Work stays primary" }
                                p { "Agent configuration opens separately so the board keeps its visual hierarchy." }
                            }
                            button {
                                class: "secondary-action",
                                onclick: move |_| {
                                    active_workspace.set(Some(WorkspacePage::AgentProfiles));
                                    focus_page_start();
                                },
                                "Manage agent profiles"
                            }
                        }
                        WorkItemControl {
                            work_items: snapshot.work_items.clone(),
                            agent_profiles: snapshot.agent_profiles.clone(),
                            portfolio: snapshot.portfolio.clone(),
                            on_transition,
                            on_agent_plan,
                            new_work_item_project_id: new_work_item_form.project_id,
                            new_work_item_id: new_work_item_form.work_item_id,
                            new_work_item_title: new_work_item_form.title,
                            new_work_item_priority: new_work_item_form.priority,
                            on_create: on_work_item_create,
                        }
                    },
                    AppPage::Projects => rsx! {
                        section { class: "page-toolbar",
                            div {
                                strong { "Project health first" }
                                p { "Open the Control Graph workspace only when routing needs attention." }
                            }
                            button {
                                class: "secondary-action",
                                onclick: move |_| {
                                    active_workspace.set(Some(WorkspacePage::ProjectGraphs));
                                    focus_page_start();
                                },
                                "Open all project graphs"
                            }
                        }
                        ProjectGrid {
                            portfolio: snapshot.portfolio.clone(),
                            execution_workspaces: snapshot.execution_workspaces.clone(),
                            control_graphs: snapshot.control_graphs.clone(),
                            graph_canvas_layouts: snapshot.graph_canvas_layouts.clone(),
                            agent_profiles: snapshot.agent_profiles.clone(),
                            project_graph_bindings: snapshot.project_graph_bindings.clone(),
                            graph_rewrite_proposals: snapshot.graph_rewrite_proposals.clone(),
                            work_item_graph_positions: snapshot.work_item_graph_positions.clone(),
                            route_decisions: snapshot.route_decisions.clone(),
                            on_connect: on_execution_workspace,
                            on_graph_select,
                            on_graph_canvas_layout,
                            on_graph_rewrite_propose,
                            on_graph_rewrite_decide,
                            on_evidence_route,
                            on_human_approval,
                            new_project_id: new_project_form.project_id,
                            new_project_name: new_project_form.name,
                            new_project_execution_cap: new_project_form.execution_cap,
                            new_project_workspace_location: new_project_form.workspace_location,
                            on_create: on_project_create,
                        }
                    },
                    AppPage::Automation => rsx! {
                        section { class: "workspace-launcher", aria_label: "Automation workspaces",
                            LaunchCard {
                                presentation: LaunchCardPresentation::Workspace,
                                kicker: "Reusable approach",
                                title: "Blueprint Studio",
                                description: "Compose typed, project-independent approaches and pin Markdown notes.",
                                meta: format!("{} revisions", snapshot.orchestration_blueprints.len()),
                                on_open: move |_| {
                                    active_workspace.set(Some(WorkspacePage::BlueprintStudio));
                                    focus_page_start();
                                },
                            }
                            LaunchCard {
                                presentation: LaunchCardPresentation::Workspace,
                                kicker: "Across products",
                                title: "Portfolio orchestration",
                                description: "Schedule project selection, bounded post-actions, approvals, and terminal routes.",
                                meta: format!("{} revisions", snapshot.portfolio_orchestrations.len()),
                                on_open: move |_| {
                                    active_workspace.set(Some(WorkspacePage::PortfolioOrchestration));
                                    focus_page_start();
                                },
                            }
                            LaunchCard {
                                presentation: LaunchCardPresentation::Workspace,
                                kicker: "Inside projects",
                                title: "Control Graph manager",
                                description: "Bind, inspect, and safely rewrite each managed project's Control Graph.",
                                meta: format!("{} managed", snapshot.portfolio.projects.len()),
                                on_open: move |_| {
                                    active_workspace.set(Some(WorkspacePage::ProjectGraphs));
                                    focus_page_start();
                                },
                            }
                        }
                    },
                    AppPage::Activity => rsx! {
                        ActivityInbox {
                            activity: snapshot.activity.clone(),
                            work_items: snapshot.work_items.clone(),
                            on_attach,
                        }
                        RecentActivity {
                            activity: snapshot.activity.clone(),
                            warning: snapshot.activity_warning.clone(),
                            on_reconcile,
                        }
                    },
                }}
                }
            }

            footer { class: "app-footer",
                span { "Local data" }
                code { "{snapshot.storage_label}" }
            }
        }
    }
}

#[component]
fn ActionNotices(notices: Vec<String>) -> Element {
    rsx! {
        for notice in notices {
            aside { class: "action-notice", "{notice}" }
        }
    }
}

fn action_notices(snapshot: &AppState) -> Vec<String> {
    [
        snapshot.reconciliation_notice.clone(),
        snapshot.attachment_notice.clone(),
        snapshot.transition_notice.clone(),
        snapshot.agent_plan_notice.clone(),
        snapshot.agent_profile_notice.clone(),
        snapshot.execution_workspace_notice.clone(),
        snapshot.project_notice.clone(),
        snapshot.graph_binding_notice.clone(),
        snapshot.graph_rewrite_notice.clone(),
        snapshot.work_item_creation_notice.clone(),
        snapshot.run_notice.clone(),
        snapshot.control_node_notice.clone(),
    ]
    .into_iter()
    .flatten()
    .collect()
}

#[component]
fn PortfolioHero(on_preview: EventHandler<MouseEvent>) -> Element {
    rsx! {
        section { class: "hero",
            div {
                p { class: "kicker", "Portfolio overview" }
                h2 { "See what every agent is doing—without opening a terminal." }
                p { class: "lede", "Board-owned coordination stays local. Recent progress is read from Core as evidence, while Work item changes remain yours to approve." }
            }
            button {
                class: "preview-action",
                title: "Preview only; Runner will not start",
                onclick: move |event| on_preview.call(event),
                "Preview next safe item"
            }
        }
    }
}

#[component]
fn ActivityInbox(
    activity: ActivityTimeline,
    work_items: Vec<WorkItemSummary>,
    on_attach: EventHandler<AttachmentRequest>,
) -> Element {
    let inbox = activity
        .activities
        .into_iter()
        .filter(ProgressActivity::is_inbox)
        .collect::<Vec<_>>();
    let inbox_count = inbox.len();
    rsx! {
        section { class: "section-heading",
            div {
                p { class: "kicker", "Needs a Work item" }
                h3 { "Activity Inbox" }
            }
            span { "{inbox_count} unlinked" }
        }

        if inbox.is_empty() {
            section { class: "inbox-clear",
                strong { "Inbox clear" }
                p { "Every recent Checkpoint is connected to a Work item." }
            }
        } else {
            section { class: "inbox-grid", aria_label: "Unlinked progress checkpoints",
                for item in inbox {
                    InboxCard {
                        key: "{item.checkpoint_id}",
                        item,
                        work_items: work_items.clone(),
                        on_attach,
                    }
                }
            }
        }
    }
}

#[component]
fn InboxCard(
    item: ProgressActivity,
    work_items: Vec<WorkItemSummary>,
    on_attach: EventHandler<AttachmentRequest>,
) -> Element {
    let candidates = work_items
        .into_iter()
        .filter(|work_item| work_item.project_id == item.project_id)
        .collect::<Vec<_>>();
    let initial = candidates
        .first()
        .map(|work_item| work_item.id.clone())
        .unwrap_or_default();
    let mut selected = use_signal(move || initial);
    let suggested_id = suggest_work_item_id(&item.project_id, &candidates);
    let suggested_title = item.summary.clone();
    let mut new_work_item_id = use_signal(move || suggested_id);
    let mut new_work_item_title = use_signal(move || suggested_title);
    let selected_id = selected.read().clone();
    let new_id = new_work_item_id.read().clone();
    let new_title = new_work_item_title.read().clone();
    let can_attach = !selected_id.is_empty();
    let can_create = !new_id.trim().is_empty() && !new_title.trim().is_empty();
    let existing_request = AttachmentRequest {
        checkpoint_id: item.checkpoint_id.clone(),
        project_id: item.project_id.clone(),
        checkpoint_work_item_id: item.work_item_id.clone(),
        target: AttachmentTarget::Existing {
            work_item_id: selected_id.clone(),
        },
    };
    let new_request = AttachmentRequest {
        checkpoint_id: item.checkpoint_id.clone(),
        project_id: item.project_id.clone(),
        checkpoint_work_item_id: item.work_item_id.clone(),
        target: AttachmentTarget::New {
            work_item_id: new_id.clone(),
            title: new_title.clone(),
        },
    };
    rsx! {
        article { class: "inbox-card",
            div { class: "activity-head",
                div { class: "activity-tags",
                    span { class: outcome_class(item.outcome), "{item.outcome.label()}" }
                    span { class: "source", "{item.source.label()}" }
                }
                time { "{display_time(&item.recorded_at)}" }
            }
            h4 { "{item.summary}" }
            p { class: "activity-link", "{item.project_id} · Unlinked Checkpoint" }
            if let Some(recommended_state) = item.recommended_state {
                span { class: "recommendation", "Suggested: {recommended_state}" }
            }
            if candidates.is_empty() {
                p { class: "inbox-unavailable",
                    "This project has no existing Work items yet."
                }
            } else {
                div { class: "attachment-actions",
                    label {
                        span { "Attach to" }
                        select {
                            value: "{selected_id}",
                            onchange: move |event| selected.set(event.value()),
                            for candidate in &candidates {
                                option { value: "{candidate.id}",
                                    "{candidate.id} · {candidate.title} · {candidate.state}"
                                }
                            }
                        }
                    }
                    button {
                        class: "attach-action",
                        disabled: !can_attach,
                        onclick: move |_| on_attach.call(existing_request.clone()),
                        "Attach"
                    }
                }
            }
            div { class: "inbox-divider", span { "or create a Work item" } }
            div { class: "creation-actions",
                label {
                    span { "Work item ID" }
                    input {
                        maxlength: 128,
                        value: "{new_id}",
                        oninput: move |event| new_work_item_id.set(event.value()),
                    }
                }
                label { class: "creation-title",
                    span { "Title" }
                    input {
                        maxlength: 256,
                        value: "{new_title}",
                        oninput: move |event| new_work_item_title.set(event.value()),
                    }
                }
                button {
                    class: "create-action",
                    disabled: !can_create,
                    onclick: move |_| on_attach.call(new_request.clone()),
                    "Create & attach"
                }
            }
            p { class: "creation-hint",
                "The new Work item starts in todo. Suggested state remains yours to review."
            }
        }
    }
}

fn suggest_work_item_id(project_id: &str, candidates: &[WorkItemSummary]) -> String {
    let existing_prefix = candidates.iter().find_map(|work_item| {
        let (prefix, suffix) = work_item.id.rsplit_once('-')?;
        suffix.parse::<u32>().ok().map(|_| prefix.to_owned())
    });
    let prefix = existing_prefix.unwrap_or_else(|| {
        let derived = project_id
            .split(|character: char| !character.is_ascii_alphanumeric())
            .rfind(|segment| !segment.is_empty())
            .unwrap_or("WORK")
            .chars()
            .take(32)
            .collect::<String>()
            .to_ascii_uppercase();
        if derived.is_empty() {
            "WORK".to_owned()
        } else {
            derived
        }
    });
    let next = candidates
        .iter()
        .filter_map(|work_item| {
            let (candidate_prefix, suffix) = work_item.id.rsplit_once('-')?;
            (candidate_prefix == prefix)
                .then(|| suffix.parse::<u32>().ok())
                .flatten()
        })
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    format!("{prefix}-{next}")
}

#[component]
fn RecentActivity(
    activity: ActivityTimeline,
    warning: Option<String>,
    on_reconcile: EventHandler<ReconciliationRequest>,
) -> Element {
    let has_older = activity.has_older;
    let linked = activity
        .activities
        .into_iter()
        .filter(|activity| !activity.is_inbox())
        .collect::<Vec<_>>();
    let linked_count = linked.len();
    rsx! {
        section { class: "section-heading",
            div {
                p { class: "kicker", "Evidence from Core" }
                h3 { "Recent activity" }
            }
            span { "{linked_count} linked checkpoints" }
        }

        if let Some(warning) = &warning {
            aside { class: "notice", "Progress history: {warning}" }
        }

        if linked.is_empty() {
            section { class: "empty-activity",
                strong { "No linked activity yet" }
                p { "Attach an Inbox Checkpoint or record progress with an active Work item." }
            }
        } else {
            section { class: "timeline", aria_label: "Recent progress checkpoints",
                for item in &linked {
                    article { class: "activity-card", key: "{item.checkpoint_id}",
                        div { class: "activity-marker" }
                        div { class: "activity-body",
                            div { class: "activity-head",
                                div { class: "activity-tags",
                                    span { class: outcome_class(item.outcome), "{item.outcome.label()}" }
                                    span { class: "source", "{item.source.label()}" }
                                }
                                time { "{display_time(&item.recorded_at)}" }
                            }
                            h4 { "{item.summary}" }
                            p { class: "activity-link",
                                "{item.project_id}"
                                if let Some(work_item_id) = item.effective_work_item_id() {
                                    span { " · {work_item_id}" }
                                }
                                if item.attachment.is_some() {
                                    span { class: "attachment-label", " · attached in Board" }
                                }
                            }
                            div { class: "activity-details",
                                if let Some(recommended_state) = item.recommended_state {
                                    span { class: "recommendation", "Suggested: {recommended_state}" }
                                }
                                for delivery in &item.deliveries {
                                    span { class: delivery_class(delivery.status),
                                        "{delivery.destination_id}: {delivery.status.label()}"
                                        if delivery.attempts > 0 {
                                            " · {delivery.attempts} attempt(s)"
                                        }
                                    }
                                }
                            }
                            for delivery in item.deliveries.iter().filter(|delivery| delivery.status.needs_attention()) {
                                if let Some(last_error) = &delivery.last_error {
                                    p { class: "delivery-error", "{delivery.destination_id}: {last_error}" }
                                }
                            }
                            ReconciliationPanel {
                                checkpoint_id: item.checkpoint_id.clone(),
                                project_id: item.project_id.clone(),
                                work_item_id: item.effective_work_item_id().map(str::to_owned),
                                recommended_state: item.recommended_state,
                                reconciliation: item.reconciliation.clone(),
                                on_reconcile,
                            }
                        }
                    }
                }
            }
            if has_older {
                p { class: "older-note", "Older checkpoints are available in Core." }
            }
        }
    }
}

#[component]
fn ReconciliationPanel(
    checkpoint_id: String,
    project_id: String,
    work_item_id: Option<String>,
    recommended_state: Option<WorkItemState>,
    reconciliation: Option<CheckpointReconciliation>,
    on_reconcile: EventHandler<ReconciliationRequest>,
) -> Element {
    if let Some(reconciliation) = reconciliation {
        let detail = if reconciliation.decision == ReconciliationDecision::Accepted
            && reconciliation.previous_state != reconciliation.resulting_state
        {
            format!(
                "{} → {}",
                reconciliation.previous_state, reconciliation.resulting_state
            )
        } else {
            format!("State: {}", reconciliation.resulting_state)
        };
        return rsx! {
            div { class: reconciliation_class(reconciliation.decision),
                strong { "{reconciliation.decision.label()}" }
                span { "{detail}" }
            }
        };
    }
    let (Some(work_item_id), Some(recommended_state)) = (work_item_id, recommended_state) else {
        return rsx! {};
    };
    if !recommended_state.is_reconciliation_target() {
        return rsx! {
            p { class: "reconciliation-unavailable",
                "This suggestion requires a separate Work item action."
            }
        };
    }

    let accepted = ReconciliationRequest {
        checkpoint_id: checkpoint_id.clone(),
        project_id: project_id.clone(),
        work_item_id: work_item_id.clone(),
        recommended_state,
        decision: ReconciliationDecision::Accepted,
    };
    let dismissed = ReconciliationRequest {
        checkpoint_id,
        project_id,
        work_item_id,
        recommended_state,
        decision: ReconciliationDecision::Dismissed,
    };
    let accept_handler = on_reconcile;
    rsx! {
        div { class: "reconciliation-actions",
            p { "Apply this suggestion to the linked Work item?" }
            div {
                button {
                    class: "accept-action",
                    onclick: move |_| accept_handler.call(accepted.clone()),
                    "Accept"
                }
                button {
                    class: "dismiss-action",
                    onclick: move |_| on_reconcile.call(dismissed.clone()),
                    "Dismiss"
                }
            }
        }
    }
}

fn reconciliation_class(decision: ReconciliationDecision) -> &'static str {
    match decision {
        ReconciliationDecision::Accepted => "reconciliation-result reconciliation-accepted",
        ReconciliationDecision::Dismissed => "reconciliation-result reconciliation-dismissed",
    }
}

#[component]
fn AgentProfileCatalog(
    agent_profiles: Vec<AgentProfileSummary>,
    work_items: Vec<WorkItemSummary>,
    portfolio: PortfolioSnapshot,
    execution_workspaces: Vec<ExecutionWorkspaceConnection>,
    new_profile_id: Signal<String>,
    new_profile_role: Signal<String>,
    new_profile_capabilities: Signal<String>,
    new_profile_instruction_ref: Signal<String>,
    new_profile_skill_refs: Signal<String>,
    on_save: EventHandler<AgentProfileSaveRequest>,
) -> Element {
    let profile_count = agent_profiles.len();
    let capability_catalog = agent_capability_catalog(&agent_profiles, &work_items);
    let capability_catalog_label = capability_catalog.join(", ");
    let first_project_id = portfolio
        .projects
        .first()
        .map(|project| project.id.clone())
        .unwrap_or_default();
    let mut inspection_project_id = use_signal(move || first_project_id);
    let selected_project_id = inspection_project_id.read().clone();
    let selected_project_name = portfolio
        .projects
        .iter()
        .find(|project| project.id == selected_project_id)
        .map_or("Unknown project", |project| project.name.as_str());
    let selected_connection = execution_workspaces
        .iter()
        .find(|connection| connection.project_id == selected_project_id);
    let behavior_inspector =
        selected_connection.map_or_else(AgentBehaviorInspector::unavailable, |connection| {
            match (connection.kind, connection.location.as_deref()) {
                (ExecutionWorkspaceKind::BundledSample, None) => {
                    AgentBehaviorInspector::bundled_sample()
                }
                (ExecutionWorkspaceKind::LocalDirectory, Some(location)) => {
                    AgentBehaviorInspector::for_workspace(std::path::Path::new(location))
                }
                _ => AgentBehaviorInspector::unavailable(),
            }
        });
    let behavior_source_label = behavior_inspector.label();
    rsx! {
        section { class: "section-heading",
            div {
                p { class: "kicker", "Scheduling roles" }
                h3 { "Agent profiles" }
            }
            span { "{profile_count} configured" }
        }
        section { class: "agent-catalog", aria_label: "Agent profile catalog",
            div { class: "behavior-inspection-source",
                label { class: "inspection-project-picker",
                    span { "Reference project" }
                    select {
                        aria_label: "Reference inspection project",
                        value: "{selected_project_id}",
                        onchange: move |event| inspection_project_id.set(event.value()),
                        for project in &portfolio.projects {
                            option { value: "{project.id}", "{project.name}" }
                        }
                    }
                }
                strong { "{behavior_source_label}" }
                small {
                    "{selected_project_name} · Presence check only; trust and Runner preflight remain separate."
                }
            }
            NewAgentProfileForm {
                existing_profiles: agent_profiles.clone(),
                profile_id: new_profile_id,
                role: new_profile_role,
                capability_input: new_profile_capabilities,
                instruction_ref_input: new_profile_instruction_ref,
                skill_ref_input: new_profile_skill_refs,
                on_save,
            }
            div { class: "agent-profile-grid",
                for profile in agent_profiles {
                    AgentProfileCard {
                        key: "{profile.id}",
                        inspection: behavior_inspector.inspect(&profile),
                        profile,
                        on_save,
                    }
                }
            }
            if capability_catalog.is_empty() {
                p { class: "agent-catalog-hint", "No scheduling capabilities are configured yet." }
            } else {
                p { class: "agent-catalog-hint",
                    "Capabilities in use: {capability_catalog_label}"
                }
            }
        }
    }
}

#[component]
fn NewAgentProfileForm(
    existing_profiles: Vec<AgentProfileSummary>,
    mut profile_id: Signal<String>,
    mut role: Signal<String>,
    mut capability_input: Signal<String>,
    mut instruction_ref_input: Signal<String>,
    mut skill_ref_input: Signal<String>,
    on_save: EventHandler<AgentProfileSaveRequest>,
) -> Element {
    let profile_id_value = profile_id.read().clone();
    let role_value = role.read().clone();
    let capability_input_value = capability_input.read().clone();
    let instruction_ref_input_value = instruction_ref_input.read().clone();
    let skill_ref_input_value = skill_ref_input.read().clone();
    let capabilities = parse_agent_capabilities(&capability_input_value);
    let instruction_ref = parse_agent_instruction_ref(&instruction_ref_input_value);
    let skill_refs = parse_agent_skill_refs(&skill_ref_input_value);
    let duplicate_id = existing_profiles
        .iter()
        .any(|profile| profile.id == profile_id_value);
    let can_create = agent_profile_id_is_valid(&profile_id_value)
        && agent_role_is_valid(&role_value)
        && capability_input_is_valid(&capability_input_value)
        && instruction_ref_input_is_valid(&instruction_ref_input_value)
        && skill_ref_input_is_valid(&skill_ref_input_value)
        && !duplicate_id;
    let request = AgentProfileSaveRequest {
        expected: None,
        target: AgentProfileSummary {
            id: profile_id_value.clone(),
            role: role_value.trim().to_owned(),
            capabilities,
            instruction_ref,
            skill_refs,
        },
    };
    rsx! {
        details { class: "new-agent-profile",
            summary { "Add Agent profile" }
            div { class: "agent-profile-form",
                label {
                    span { "Stable ID" }
                    input {
                        aria_label: "New Agent profile ID",
                        value: "{profile_id_value}",
                        maxlength: 64,
                        placeholder: "qa-specialist",
                        oninput: move |event| profile_id.set(event.value()),
                    }
                    small { "Lowercase letters, numbers, hyphens, or underscores." }
                }
                label {
                    span { "Role name" }
                    input {
                        aria_label: "New Agent profile role",
                        value: "{role_value}",
                        maxlength: 128,
                        placeholder: "QA specialist",
                        oninput: move |event| role.set(event.value()),
                    }
                }
                AgentCapabilityInput {
                    value: capability_input_value,
                    label: "Declared capabilities",
                    on_change: move |value| capability_input.set(value),
                }
                AgentInstructionInput {
                    value: instruction_ref_input_value,
                    on_change: move |value| instruction_ref_input.set(value),
                }
                AgentSkillInput {
                    value: skill_ref_input_value,
                    on_change: move |value| skill_ref_input.set(value),
                }
                if duplicate_id {
                    p { class: "field-warning", "That Agent profile ID already exists." }
                }
                button {
                    class: "agent-profile-action",
                    disabled: !can_create,
                    onclick: move |_| on_save.call(request.clone()),
                    "Create profile"
                }
            }
        }
    }
}

#[component]
fn AgentProfileCard(
    profile: AgentProfileSummary,
    inspection: AgentBehaviorInspection,
    on_save: EventHandler<AgentProfileSaveRequest>,
) -> Element {
    let initial_role = profile.role.clone();
    let initial_capabilities = profile.capabilities.join(", ");
    let initial_instruction_ref = profile.instruction_ref.clone().unwrap_or_default();
    let initial_skill_refs = profile.skill_refs.join(", ");
    let mut role = use_signal(move || initial_role);
    let mut capability_input = use_signal(move || initial_capabilities);
    let mut instruction_ref_input = use_signal(move || initial_instruction_ref);
    let mut skill_ref_input = use_signal(move || initial_skill_refs);
    let role_value = role.read().clone();
    let capability_input_value = capability_input.read().clone();
    let instruction_ref_input_value = instruction_ref_input.read().clone();
    let skill_ref_input_value = skill_ref_input.read().clone();
    let target = AgentProfileSummary {
        id: profile.id.clone(),
        role: role_value.trim().to_owned(),
        capabilities: parse_agent_capabilities(&capability_input_value),
        instruction_ref: parse_agent_instruction_ref(&instruction_ref_input_value),
        skill_refs: parse_agent_skill_refs(&skill_ref_input_value),
    };
    let can_save = target != profile
        && agent_role_is_valid(&role_value)
        && capability_input_is_valid(&capability_input_value)
        && instruction_ref_input_is_valid(&instruction_ref_input_value)
        && skill_ref_input_is_valid(&skill_ref_input_value);
    let request = AgentProfileSaveRequest {
        expected: Some(profile.clone()),
        target,
    };
    let capability_label = if profile.capabilities.is_empty() {
        "No capabilities".to_owned()
    } else {
        profile.capabilities.join(", ")
    };
    let instruction_label = profile
        .instruction_ref
        .as_deref()
        .unwrap_or("Not configured");
    let skill_label = if profile.skill_refs.is_empty() {
        "No Skills".to_owned()
    } else {
        profile.skill_refs.join(", ")
    };
    rsx! {
        article { class: "agent-profile-card",
            div { class: "agent-profile-card-head",
                div {
                    h4 { "{profile.role}" }
                    code { "{profile.id}" }
                }
                span { "{profile.capabilities.len()} capabilities" }
            }
            p { class: "agent-profile-capabilities", "{capability_label}" }
            dl { class: "agent-profile-behavior",
                div {
                    dt { "Instructions" }
                    dd { code { "{instruction_label}" } }
                }
                div {
                    dt { "Skills" }
                    dd { "{skill_label}" }
                }
            }
            AgentBehaviorInspectionView { inspection }
            details {
                summary { "Edit profile" }
                div { class: "agent-profile-form compact",
                    label {
                        span { "Role name" }
                        input {
                            aria_label: "Role for {profile.id}",
                            value: "{role_value}",
                            maxlength: 128,
                            oninput: move |event| role.set(event.value()),
                        }
                    }
                    AgentCapabilityInput {
                        value: capability_input_value,
                        label: "Declared capabilities",
                        on_change: move |value| capability_input.set(value),
                    }
                    AgentInstructionInput {
                        value: instruction_ref_input_value,
                        on_change: move |value| instruction_ref_input.set(value),
                    }
                    AgentSkillInput {
                        value: skill_ref_input_value,
                        on_change: move |value| skill_ref_input.set(value),
                    }
                    button {
                        class: "agent-profile-action",
                        disabled: !can_save,
                        onclick: move |_| on_save.call(request.clone()),
                        "Save profile"
                    }
                }
            }
        }
    }
}

#[component]
fn AgentBehaviorInspectionView(inspection: AgentBehaviorInspection) -> Element {
    let status_label = behavior_inspection_status_label(inspection.status);
    let status_class = behavior_inspection_status_class(inspection.status);
    let class = format!("behavior-inspection {status_class}");
    let unavailable = inspection.status == BehaviorInspectionStatus::WorkspaceUnavailable;
    let no_references = inspection.status == BehaviorInspectionStatus::NoReferences;
    rsx! {
        div { class: "{class}",
            div { class: "behavior-inspection-head",
                strong { "Reference check" }
                span { "{status_label}" }
            }
            if unavailable {
                p { "The selected Execution workspace is unavailable. References were not inspected." }
            } else if no_references {
                p { "No instruction or Skill references are configured." }
            } else {
                ul {
                    if let Some(instruction) = inspection.instruction {
                        AgentBehaviorReferenceRow {
                            kind: "Instructions",
                            inspection: instruction,
                        }
                    }
                    for skill in inspection.skills {
                        AgentBehaviorReferenceRow {
                            key: "{skill.reference}",
                            kind: "Skill",
                            inspection: skill,
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn AgentBehaviorReferenceRow(kind: &'static str, inspection: ReferenceInspection) -> Element {
    let status_label = reference_status_label(inspection.status);
    let status_class = reference_status_class(inspection.status);
    rsx! {
        li {
            span { "{kind}" }
            code { "{inspection.reference}" }
            em { class: "{status_class}", "{status_label}" }
        }
    }
}

const fn behavior_inspection_status_label(status: BehaviorInspectionStatus) -> &'static str {
    match status {
        BehaviorInspectionStatus::Ready => "Present",
        BehaviorInspectionStatus::NeedsAttention => "Needs attention",
        BehaviorInspectionStatus::Unsafe => "Unsafe",
        BehaviorInspectionStatus::WorkspaceUnavailable => "Workspace unavailable",
        BehaviorInspectionStatus::NoReferences => "No references",
    }
}

const fn behavior_inspection_status_class(status: BehaviorInspectionStatus) -> &'static str {
    match status {
        BehaviorInspectionStatus::Ready => "inspection-ready",
        BehaviorInspectionStatus::NeedsAttention => "inspection-missing",
        BehaviorInspectionStatus::Unsafe => "inspection-unsafe",
        BehaviorInspectionStatus::WorkspaceUnavailable => "inspection-unavailable",
        BehaviorInspectionStatus::NoReferences => "inspection-empty",
    }
}

const fn reference_status_label(status: ReferenceStatus) -> &'static str {
    match status {
        ReferenceStatus::Present => "Present",
        ReferenceStatus::Missing => "Missing",
        ReferenceStatus::Unsafe => "Unsafe",
        ReferenceStatus::NotInspected => "Not inspected",
    }
}

const fn reference_status_class(status: ReferenceStatus) -> &'static str {
    match status {
        ReferenceStatus::Present => "reference-present",
        ReferenceStatus::Missing => "reference-missing",
        ReferenceStatus::Unsafe => "reference-unsafe",
        ReferenceStatus::NotInspected => "reference-unavailable",
    }
}

#[component]
fn AgentCapabilityInput(
    value: String,
    label: &'static str,
    on_change: EventHandler<String>,
) -> Element {
    rsx! {
        label {
            span { "{label}" }
            input {
                aria_label: "{label}",
                value: "{value}",
                maxlength: 512,
                placeholder: "research, evidence",
                oninput: move |event| on_change.call(event.value()),
            }
            small { "Separate capabilities with commas. Leave blank for none." }
        }
    }
}

#[component]
fn AgentInstructionInput(value: String, on_change: EventHandler<String>) -> Element {
    rsx! {
        label {
            span { "Instruction file" }
            input {
                aria_label: "Agent instruction reference",
                value: "{value}",
                maxlength: 512,
                placeholder: "agents/researcher/AGENT.md",
                oninput: move |event| on_change.call(event.value()),
            }
            small {
                "Optional path relative to the Execution workspace. The file is resolved before a Run."
            }
        }
    }
}

#[component]
fn AgentSkillInput(value: String, on_change: EventHandler<String>) -> Element {
    rsx! {
        label {
            span { "Skill references" }
            input {
                aria_label: "Agent Skill references",
                value: "{value}",
                maxlength: 512,
                placeholder: "web-research, evidence-summary",
                oninput: move |event| on_change.call(event.value()),
            }
            small {
                "Stable Skill IDs separated with commas. References do not install or enable a Skill."
            }
        }
    }
}

#[component]
fn WorkItemControl(
    work_items: Vec<WorkItemSummary>,
    agent_profiles: Vec<AgentProfileSummary>,
    portfolio: PortfolioSnapshot,
    on_transition: EventHandler<WorkItemTransitionRequest>,
    on_agent_plan: EventHandler<AgentPlanUpdateRequest>,
    new_work_item_project_id: Signal<String>,
    new_work_item_id: Signal<String>,
    new_work_item_title: Signal<String>,
    new_work_item_priority: Signal<String>,
    on_create: EventHandler<WorkItemCreateRequest>,
) -> Element {
    let mut project_filter = use_signal(String::new);
    let mut selected_work_item = use_signal(|| None::<WorkItemSummary>);
    let mut dragged_work_item = use_signal(|| None::<WorkItemSummary>);
    let mut drag_origin = use_signal(|| None::<CanvasPoint>);
    let mut pointer_drag_active = use_signal(|| false);
    let mut drag_cursor = use_signal(|| None::<CanvasPoint>);
    let project_filter_value = project_filter.read().clone();
    let selected_work_item_value = selected_work_item.read().clone();
    let visible_work_items = filter_work_items_by_project(&work_items, &project_filter_value);
    let work_item_count = visible_work_items.len();
    let capability_catalog = agent_capability_catalog(&agent_profiles, &work_items);
    let lanes = group_work_items_by_state(visible_work_items);
    let kanban_class = if *pointer_drag_active.read() {
        "kanban-board dragging"
    } else {
        "kanban-board"
    };
    let drag_feedback = if *pointer_drag_active.read() {
        dragged_work_item
            .read()
            .clone()
            .zip(*drag_cursor.read())
            .map(|(item, position)| (position, item.id, item.title))
    } else {
        None
    };
    rsx! {
        section { class: "section-heading",
            div {
                p { class: "kicker", "State authority" }
                h3 { "Work item Kanban" }
            }
            span { "{work_item_count} total" }
        }

        div { class: "kanban-filter",
            label {
                span { "Show" }
                select {
                    aria_label: "Kanban project filter",
                    value: "{project_filter_value}",
                    onchange: move |event| project_filter.set(event.value()),
                    option { value: "", "All projects" }
                    for project in &portfolio.projects {
                        option {
                            value: "{project.id}",
                            selected: project_filter_value == project.id,
                            "{project.name} · {project.id}"
                        }
                    }
                }
            }
            small { "Filters this Kanban view only; Work item state is unchanged." }
        }

        NewWorkItemForm {
            portfolio: portfolio.clone(),
            existing_work_items: work_items.clone(),
            project_id: new_work_item_project_id,
            work_item_id: new_work_item_id,
            title: new_work_item_title,
            priority: new_work_item_priority,
            on_create,
        }

        if work_item_count == 0 {
            section { class: "empty-work-items",
                strong { "No Work items in this view" }
                p { "Add a direct Work item or create one from an Activity Inbox Checkpoint." }
            }
        } else {
            p { class: "kanban-scroll-hint",
                strong { "Move work visually" }
                span { "Drag cards between lanes. Open Details for the keyboard-friendly state controls. Scroll sideways to reach later states." }
            }
            section {
                class: "{kanban_class}",
                aria_label: "Work item Kanban board",
                onpointermove: move |event| {
                    if let Some(origin) = *drag_origin.read() {
                        let current = event.client_coordinates();
                        let current = CanvasPoint { x: current.x, y: current.y };
                        if *pointer_drag_active.read() || pointer_drag_exceeded(origin, current) {
                            pointer_drag_active.set(true);
                            drag_cursor.set(Some(current));
                        }
                    }
                },
                onpointerup: move |_| {
                    dragged_work_item.set(None);
                    drag_origin.set(None);
                    pointer_drag_active.set(false);
                    drag_cursor.set(None);
                },
                onpointercancel: move |_| {
                    dragged_work_item.set(None);
                    drag_origin.set(None);
                    pointer_drag_active.set(false);
                    drag_cursor.set(None);
                },
                for (state, lane_items) in lanes {
                    KanbanLane {
                        key: "{state.as_str()}",
                        state,
                        work_items: lane_items,
                        selected_work_item,
                        dragged_work_item,
                        drag_origin,
                        pointer_drag_active,
                        drag_cursor,
                        on_transition,
                    }
                }
            }
            if let Some((position, label, detail)) = drag_feedback {
                PointerDragGhost {
                    position,
                    label,
                    detail,
                    variant: "work-item".to_owned(),
                }
            }
        }
        if let Some(item) = selected_work_item_value {
            WorkItemDetailDialog {
                item,
                agent_profiles: agent_profiles.clone(),
                capability_catalog: capability_catalog.clone(),
                on_transition,
                on_agent_plan,
                on_close: move |()| selected_work_item.set(None),
            }
        }
    }
}

#[component]
fn NewWorkItemForm(
    portfolio: PortfolioSnapshot,
    existing_work_items: Vec<WorkItemSummary>,
    mut project_id: Signal<String>,
    mut work_item_id: Signal<String>,
    mut title: Signal<String>,
    mut priority: Signal<String>,
    on_create: EventHandler<WorkItemCreateRequest>,
) -> Element {
    let project_id_value = project_id.read().clone();
    let work_item_id_value = work_item_id.read().clone();
    let title_value = title.read().clone();
    let priority_value = priority.read().clone();
    let selected_project_id = if project_id_value.is_empty() {
        portfolio
            .projects
            .first()
            .map(|project| project.id.clone())
            .unwrap_or_default()
    } else {
        project_id_value.clone()
    };
    let duplicate_id = existing_work_items
        .iter()
        .any(|work_item| work_item.id == work_item_id_value);
    let parsed_priority = parse_work_item_priority(&priority_value);
    let can_create = !selected_project_id.is_empty()
        && work_item_id_is_valid(&work_item_id_value)
        && work_item_title_is_valid(&title_value)
        && parsed_priority.is_some()
        && !duplicate_id;
    rsx! {
        details { class: "new-work-item",
            summary { "Add Work item" }
            div { class: "work-item-create-form",
                label {
                    span { "Project" }
                    select {
                        aria_label: "New Work item project",
                        value: "{selected_project_id}",
                        onchange: move |event| project_id.set(event.value()),
                        for project in &portfolio.projects {
                            option {
                                value: "{project.id}",
                                selected: selected_project_id == project.id,
                                "{project.name} · {project.id}"
                            }
                        }
                    }
                }
                label {
                    span { "Work item ID" }
                    input {
                        aria_label: "New Work item ID",
                        value: "{work_item_id_value}",
                        maxlength: 128,
                        placeholder: "PRODUCT-1",
                        oninput: move |event| work_item_id.set(event.value()),
                    }
                }
                label {
                    span { "Title" }
                    input {
                        aria_label: "New Work item title",
                        value: "{title_value}",
                        maxlength: 256,
                        placeholder: "Describe the next outcome",
                        oninput: move |event| title.set(event.value()),
                    }
                }
                label {
                    span { "Priority" }
                    input {
                        aria_label: "New Work item priority",
                        r#type: "number",
                        min: "1",
                        max: "4294967295",
                        value: "{priority_value}",
                        oninput: move |event| priority.set(event.value()),
                    }
                    small { "Lower values are selected first. New items start in todo." }
                }
                if duplicate_id {
                    p { class: "field-warning", "That Work item ID already exists." }
                }
                button {
                    class: "work-item-create-action",
                    disabled: !can_create,
                    onclick: move |_| {
                        if let Some(priority) = parsed_priority {
                            on_create.call(WorkItemCreateRequest {
                                project_id: selected_project_id.clone(),
                                work_item_id: work_item_id_value.clone(),
                                title: title_value.trim().to_owned(),
                                priority,
                            });
                        }
                    },
                    "Add Work item"
                }
            }
        }
    }
}

#[component]
fn KanbanLane(
    state: WorkItemState,
    work_items: Vec<WorkItemSummary>,
    mut selected_work_item: Signal<Option<WorkItemSummary>>,
    mut dragged_work_item: Signal<Option<WorkItemSummary>>,
    mut drag_origin: Signal<Option<CanvasPoint>>,
    mut pointer_drag_active: Signal<bool>,
    mut drag_cursor: Signal<Option<CanvasPoint>>,
    on_transition: EventHandler<WorkItemTransitionRequest>,
) -> Element {
    let mut is_drag_over = use_signal(|| false);
    let item_count = work_items.len();
    let state_label = work_item_state_label(state);
    let item_noun = if item_count == 1 { "item" } else { "items" };
    let is_drop_target = *pointer_drag_active.read()
        && *is_drag_over.read()
        && dragged_work_item
            .read()
            .as_ref()
            .is_some_and(|item| item.state != state);
    let lane_class = if is_drop_target {
        format!("{} drop-target", kanban_lane_class(state))
    } else {
        kanban_lane_class(state).to_owned()
    };
    rsx! {
        section {
            class: "{lane_class}",
            aria_label: "{state_label} lane, {item_count} {item_noun}",
            onpointerenter: move |_| {
                if *pointer_drag_active.read() && dragged_work_item.read().is_some() {
                    is_drag_over.set(true);
                }
            },
            onpointerleave: move |_| is_drag_over.set(false),
            onpointerup: move |_| {
                let was_dragging = *pointer_drag_active.read();
                let item = dragged_work_item.read().clone();
                dragged_work_item.set(None);
                drag_origin.set(None);
                pointer_drag_active.set(false);
                drag_cursor.set(None);
                is_drag_over.set(false);
                if was_dragging
                    && let Some(request) = item
                        .as_ref()
                        .and_then(|item| work_item_drop_transition(item, state))
                {
                    on_transition.call(request);
                }
            },
            header { class: "kanban-lane-head",
                div { class: "kanban-lane-title",
                    span { class: "kanban-lane-marker", aria_hidden: "true" }
                    h4 { "{state_label}" }
                }
                span { class: "kanban-count", "{item_count}" }
            }
            p { class: "kanban-lane-hint", "{work_item_eligibility_label(state)}" }
            div { class: "kanban-lane-items",
                if work_items.is_empty() {
                    p { class: "kanban-empty", "No items" }
                } else {
                    for item in work_items {
                        WorkItemCard {
                            key: "{item.id}",
                            item,
                            selected_work_item,
                            dragged_work_item,
                            drag_origin,
                            pointer_drag_active,
                            drag_cursor,
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn WorkItemCard(
    item: WorkItemSummary,
    mut selected_work_item: Signal<Option<WorkItemSummary>>,
    mut dragged_work_item: Signal<Option<WorkItemSummary>>,
    mut drag_origin: Signal<Option<CanvasPoint>>,
    mut pointer_drag_active: Signal<bool>,
    mut drag_cursor: Signal<Option<CanvasPoint>>,
) -> Element {
    let item_for_pointer = item.clone();
    let item_for_open = item.clone();
    let approval_needed = item.approval_requirement == ApprovalRequirement::Explicit;
    let dependency_count = item.dependency_ids.len();
    let is_unassigned = item.agent_profile_id.is_none();
    let is_dragging = *pointer_drag_active.read()
        && dragged_work_item
            .read()
            .as_ref()
            .is_some_and(|dragged| dragged.id == item.id);
    let card_class = if is_dragging {
        "work-item-card dragging"
    } else {
        "work-item-card"
    };
    rsx! {
        article {
            class: "{card_class}",
            aria_label: "{item.id}: {item.title}. Drag to another lane or open details.",
            onpointerdown: move |event| {
                let origin = event.client_coordinates();
                dragged_work_item.set(Some(item_for_pointer.clone()));
                drag_origin.set(Some(CanvasPoint { x: origin.x, y: origin.y }));
                pointer_drag_active.set(false);
                drag_cursor.set(None);
            },
            onpointercancel: move |_| {
                dragged_work_item.set(None);
                drag_origin.set(None);
                pointer_drag_active.set(false);
                drag_cursor.set(None);
            },
            button {
                class: "work-item-summary",
                r#type: "button",
                aria_label: "Open details for {item.id}",
                onclick: move |_| selected_work_item.set(Some(item_for_open.clone())),
                div { class: "work-item-summary-topline",
                    p { class: "work-item-id", "{item.id}" }
                    span { class: "work-item-priority", "P{item.priority}" }
                }
                h4 { "{item.title}" }
                p { class: "work-item-project", "{item.project_id}" }
                if approval_needed || dependency_count > 0 || is_unassigned {
                    div { class: "work-item-flags", aria_label: "Items needing attention",
                        if approval_needed {
                            span { "Approval" }
                        }
                        if dependency_count > 0 {
                            span { "{dependency_count} dependencies" }
                        }
                        if is_unassigned {
                            span { "Unassigned" }
                        }
                    }
                }
                span { class: "work-item-open-label", "Details" }
            }
        }
    }
}

#[component]
fn WorkItemDetailDialog(
    item: WorkItemSummary,
    agent_profiles: Vec<AgentProfileSummary>,
    capability_catalog: Vec<String>,
    on_transition: EventHandler<WorkItemTransitionRequest>,
    on_agent_plan: EventHandler<AgentPlanUpdateRequest>,
    on_close: EventHandler<()>,
) -> Element {
    let observed_state = item.state;
    let mut target = use_signal(move || observed_state);
    let target_state = *target.read();
    let can_update = target_state != observed_state;
    let request = WorkItemTransitionRequest {
        project_id: item.project_id.clone(),
        work_item_id: item.id.clone(),
        expected_state: observed_state,
        target_state,
    };
    let expected_agent_plan = item.agent_plan();
    let initial_agent_profile_id = expected_agent_plan.agent_profile_id.clone();
    let initial_capabilities = expected_agent_plan.required_capabilities.clone();
    let mut target_agent_profile_id = use_signal(move || initial_agent_profile_id);
    let mut target_capabilities = use_signal(move || initial_capabilities);
    let selected_agent_profile_id = target_agent_profile_id.read().clone();
    let selected_capabilities = target_capabilities.read().clone();
    let target_agent_plan = AgentPlan {
        agent_profile_id: selected_agent_profile_id.clone(),
        required_capabilities: selected_capabilities.clone(),
    };
    let can_update_agent_plan = target_agent_plan != expected_agent_plan;
    let agent_plan_request = AgentPlanUpdateRequest {
        project_id: item.project_id.clone(),
        work_item_id: item.id.clone(),
        expected: expected_agent_plan,
        target: target_agent_plan,
    };
    let on_capability_change = move |(capability, selected): (String, bool)| {
        let mut capabilities = target_capabilities.write();
        if selected {
            if !capabilities.contains(&capability) {
                capabilities.push(capability);
                capabilities.sort();
            }
        } else {
            capabilities.retain(|candidate| candidate != &capability);
        }
    };
    let agent_label = item
        .agent_profile_id
        .as_deref()
        .map_or("Unassigned", |agent_profile_id| agent_profile_id);
    let capability_label = if item.required_capabilities.is_empty() {
        "No capability requirements".to_owned()
    } else {
        format!("Requires · {}", item.required_capabilities.join(", "))
    };
    rsx! {
        DetailDialog {
            title: item.title.clone(),
            description: format!("{} · {} · Priority {}", item.id, item.project_id, item.priority),
            kicker: "Work item details".to_owned(),
            class_name: "detail-modal".to_owned(),
            on_close,
            article { class: "work-item-detail",
            div { class: "work-item-detail-state",
                span { "Current state" }
                span { class: work_item_state_class(item.state),
                    "{work_item_state_label(item.state)}"
                }
            }
            div { class: "work-item-gates",
                span { class: approval_requirement_class(item.approval_requirement),
                    "Approval · {approval_requirement_label(item.approval_requirement)}"
                }
                span {
                    if item.dependency_ids.is_empty() {
                        "No dependencies"
                    } else {
                        "{item.dependency_ids.len()} dependency(s)"
                    }
                }
                span { class: if item.agent_profile_id.is_some() { "work-item-gate gate-agent" } else { "work-item-gate gate-missing" },
                    "Agent · {agent_label}"
                }
                span { "{capability_label}" }
            }
            p { class: "work-item-eligibility", "{work_item_eligibility_label(item.state)}" }
            details { class: "agent-plan-editor",
                summary { "Edit Agent plan" }
                div { class: "agent-plan-fields",
                    label {
                        span { "Assigned Agent" }
                        select {
                            aria_label: "Assigned Agent for {item.id}",
                            value: selected_agent_profile_id.as_deref().unwrap_or(""),
                            onchange: move |event| {
                                let value = event.value();
                                target_agent_profile_id.set(if value.is_empty() {
                                    None
                                } else {
                                    Some(value)
                                });
                            },
                            option {
                                value: "",
                                selected: selected_agent_profile_id.is_none(),
                                "Unassigned"
                            }
                            for profile in agent_profiles {
                                option {
                                    value: "{profile.id}",
                                    selected: selected_agent_profile_id.as_deref() == Some(profile.id.as_str()),
                                    "{profile.role} · {profile.id}"
                                }
                            }
                        }
                    }
                    fieldset { class: "capability-picker",
                        legend { "Required capabilities" }
                        if capability_catalog.is_empty() {
                            p { "No Agent capabilities available" }
                        } else {
                            for capability in capability_catalog {
                                CapabilityToggle {
                                    key: "{capability}",
                                    capability: capability.clone(),
                                    selected: selected_capabilities.contains(&capability),
                                    on_change: on_capability_change,
                                }
                            }
                        }
                    }
                    button {
                        class: "agent-plan-action",
                        disabled: !can_update_agent_plan,
                        onclick: move |_| {
                            on_agent_plan.call(agent_plan_request.clone());
                            on_close.call(());
                        },
                        "Save Agent plan"
                    }
                }
            }
            div { class: "work-item-actions",
                label {
                    span { "Move to" }
                    select {
                        aria_label: "New state for {item.id}",
                        value: "{target_state.as_str()}",
                        onchange: move |event| {
                            if let Ok(state) = WorkItemState::try_from(event.value().as_str()) {
                                target.set(state);
                            }
                        },
                        for state in WORK_ITEM_STATES {
                            option {
                                value: "{state.as_str()}",
                                selected: state == target_state,
                                "{work_item_state_label(state)}"
                            }
                        }
                    }
                }
                button {
                    class: "transition-action",
                    disabled: !can_update,
                    onclick: move |_| {
                        on_transition.call(request.clone());
                        on_close.call(());
                    },
                    "Update state"
                }
            }
            }
        }
    }
}

#[component]
fn CapabilityToggle(
    capability: String,
    selected: bool,
    on_change: EventHandler<(String, bool)>,
) -> Element {
    let capability_for_change = capability.clone();
    rsx! {
        label { class: "capability-choice",
            input {
                r#type: "checkbox",
                checked: selected,
                onchange: move |event| {
                    on_change.call((capability_for_change.clone(), event.checked()));
                }
            }
            span { "{capability}" }
        }
    }
}

#[component]
fn AutopilotPreviewPanel(
    preview: SafeAutopilotPreview,
    run_in_progress: bool,
    graph_positions: Vec<WorkItemGraphPosition>,
    control_graphs: Vec<ControlGraphRevision>,
    on_run: EventHandler<String>,
    on_evidence_route: EventHandler<EvidenceRouteRequest>,
    on_human_approval: EventHandler<HumanApprovalRequest>,
) -> Element {
    let decision = preview.decision().as_str();
    let fast_exit = if preview.fast_exit_required() {
        "Fast exit required"
    } else {
        "Candidate available"
    };
    rsx! {
        section { class: "autopilot-preview", aria_live: "polite",
            header { class: "preview-head",
                div {
                    p { class: "kicker", "Safe Autopilot preview" }
                    h3 { "Next-action explanation" }
                }
                div { class: "preview-statuses",
                    span { class: "decision-pill", "Decision · {decision}" }
                    span { class: "preview-read-only", "Preview first · explicit start" }
                }
            }

            match &preview.outcome {
                SafeAutopilotOutcome::Candidate(candidate) => {
                    let graph_position = graph_positions
                        .iter()
                        .find(|position| position.work_item_id == candidate.work_item.id)
                        .cloned();
                    let pinned_graph = graph_position.as_ref().and_then(|position| {
                        control_graphs
                            .iter()
                            .find(|graph| {
                                graph.graph_id == position.graph_id
                                    && graph.revision_id == position.revision_id
                            })
                            .cloned()
                    });
                    let current_is_agent_loop = graph_position.as_ref().is_none_or(|position| {
                        pinned_graph.as_ref().is_some_and(|graph| {
                            graph.nodes.iter().any(|node| {
                                node.id == position.current_node_id
                                    && matches!(node.kind, ControlNodeKind::AgentLoop { .. })
                            })
                        })
                    });
                    rsx! {
                    article { class: "preview-candidate",
                        div { class: "preview-candidate-id",
                            span { "Selected candidate" }
                            strong { "{candidate.work_item.id}" }
                        }
                        div { class: "preview-candidate-body",
                            h4 { "{candidate.work_item.title}" }
                            p {
                                "{candidate.project_name} is at {candidate.active_runs}/{candidate.execution_cap} active capacity. {candidate.agent_role} is assigned and satisfies every required Agent capability. This item is priority {candidate.work_item.priority}."
                            }
                            div { class: "preview-facts",
                                span { "Board gates passed" }
                                span { "Lowest project load first" }
                                span { "Then priority" }
                                span { "Then stable IDs" }
                            }
                            if current_is_agent_loop {
                                button {
                                    class: "run-current-loop-action",
                                    disabled: run_in_progress,
                                    title: "Start this Work item's current Graph node in an isolated worktree",
                                    onclick: {
                                        let work_item_id = candidate.work_item.id.clone();
                                        move |_| on_run.call(work_item_id.clone())
                                    },
                                    if run_in_progress {
                                        "Agent Loop is running…"
                                    } else {
                                        "Start current Agent Loop"
                                    }
                                }
                            } else if let Some(position) = graph_position {
                                div { class: "preview-current-control-node",
                                    p { "This Work item is waiting at its current non-Agent Graph stage." }
                                    ControlGraphPositionCard {
                                        position,
                                        graph: pinned_graph,
                                        on_evidence_route,
                                        on_human_approval,
                                    }
                                }
                            }
                        }
                    }
                }
                },
                SafeAutopilotOutcome::NoCandidate(reason) => rsx! {
                    article { class: "preview-empty-result",
                        strong { "No candidate this tick" }
                        p { "{no_candidate_message(*reason)}" }
                    }
                },
                SafeAutopilotOutcome::Stop(reason) => rsx! {
                    article { class: "preview-stop-result",
                        strong { "Preview stopped safely" }
                        p { "{autopilot_stop_message(reason)}" }
                    }
                },
            }

            div { class: "preview-foot",
                div {
                    strong { "{fast_exit}" }
                    p { "Stored dependencies, Approval requirements, assignment, and Agent capabilities are checked here. Execution still requires trusted Core capability, workspace, cooldown, and evidence preflights." }
                }
                span { "Global capacity {PREVIEW_GLOBAL_CONCURRENCY_CAP}" }
            }

            if !preview.skipped.is_empty() {
                details { class: "preview-skips",
                    summary { "Why {preview.skipped.len()} other Work item(s) were skipped" }
                    ul {
                        for skip in &preview.skipped {
                            li { key: "{skip.work_item.project_id}-{skip.work_item.id}",
                                strong { "{skip.work_item.id}" }
                                span { "{candidate_skip_message(&skip.reason)}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn candidate_skip_message(reason: &CandidateSkipReason) -> String {
    match reason {
        CandidateSkipReason::StateNotTodo(state) => {
            format!("State is {state}; preview considers todo only.")
        }
        CandidateSkipReason::ProjectAtCapacity {
            active_runs,
            execution_cap,
        } => format!("Project capacity is full at {active_runs}/{execution_cap}."),
        CandidateSkipReason::GlobalCapacityReached {
            active_runs,
            concurrency_cap,
        } => format!("Global capacity is full at {active_runs}/{concurrency_cap}."),
        CandidateSkipReason::ApprovalRequired => {
            "Explicit human approval is required before execution.".to_owned()
        }
        CandidateSkipReason::DependencyNotDone {
            dependency_id,
            state,
        } => format!("Dependency {dependency_id} is still {state}."),
        CandidateSkipReason::AgentNotAssigned => {
            "No Agent profile is assigned to this Work item.".to_owned()
        }
        CandidateSkipReason::AgentCapabilityUnavailable {
            agent_profile_id,
            capability,
        } => format!("Agent {agent_profile_id} does not declare {capability}."),
        CandidateSkipReason::LowerRanked => {
            "Runnable, but ranked behind the selected candidate.".to_owned()
        }
    }
}

fn no_candidate_message(reason: NoCandidateReason) -> String {
    match reason {
        NoCandidateReason::GlobalCapacityReached {
            active_runs,
            concurrency_cap,
        } => format!("Global capacity is already {active_runs}/{concurrency_cap}."),
        NoCandidateReason::NoRunnableCandidate => {
            "No todo Work item passed the stored dependency, Approval, assignment, Agent capability, and capacity gates.".to_owned()
        }
    }
}

fn autopilot_stop_message(reason: &AutopilotStopReason) -> String {
    match reason {
        AutopilotStopReason::InvalidGlobalConcurrencyCap => {
            "The global concurrency cap is invalid.".to_owned()
        }
        AutopilotStopReason::InvalidProjectCapacity { project_id } => {
            format!("Project {project_id} has an invalid execution cap.")
        }
        AutopilotStopReason::DuplicateProject { project_id } => {
            format!("Project {project_id} appears more than once.")
        }
        AutopilotStopReason::DuplicateWorkItem { work_item_id } => {
            format!("Work item {work_item_id} appears more than once.")
        }
        AutopilotStopReason::DuplicateAgentProfile { agent_profile_id } => {
            format!("Agent profile {agent_profile_id} appears more than once.")
        }
        AutopilotStopReason::ProjectNotFound { project_id } => {
            format!("A Work item refers to missing project {project_id}.")
        }
        AutopilotStopReason::DependencyNotFound {
            work_item_id,
            dependency_id,
        } => format!("Work item {work_item_id} refers to missing dependency {dependency_id}."),
        AutopilotStopReason::AgentProfileNotFound {
            work_item_id,
            agent_profile_id,
        } => {
            format!("Work item {work_item_id} refers to missing Agent profile {agent_profile_id}.")
        }
    }
}

fn approval_requirement_label(requirement: ApprovalRequirement) -> &'static str {
    match requirement {
        ApprovalRequirement::None => "None",
        ApprovalRequirement::Explicit => "Explicit",
    }
}

fn approval_requirement_class(requirement: ApprovalRequirement) -> &'static str {
    match requirement {
        ApprovalRequirement::None => "work-item-gate gate-ready",
        ApprovalRequirement::Explicit => "work-item-gate gate-approval",
    }
}

fn parse_agent_capabilities(value: &str) -> Vec<String> {
    let mut capabilities = value
        .split(',')
        .map(str::trim)
        .filter(|capability| !capability.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    capabilities.sort();
    capabilities.dedup();
    capabilities
}

fn capability_input_is_valid(value: &str) -> bool {
    value.chars().count() <= 512
        && value
            .split(',')
            .map(str::trim)
            .filter(|capability| !capability.is_empty())
            .all(|capability| capability.chars().count() <= 64)
}

fn parse_agent_instruction_ref(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn instruction_ref_input_is_valid(value: &str) -> bool {
    let value = value.trim();
    value.is_empty() || agent_instruction_ref_is_valid(value)
}

fn agent_instruction_ref_is_valid(value: &str) -> bool {
    let invalid_segment = value
        .split('/')
        .any(|segment| segment.is_empty() || matches!(segment, "." | ".."));
    value.chars().count() <= 512
        && !value.starts_with('/')
        && !value.contains(['\\', ':'])
        && !value.chars().any(char::is_control)
        && !invalid_segment
}

fn parse_agent_skill_refs(value: &str) -> Vec<String> {
    let mut skill_refs = value
        .split(',')
        .map(str::trim)
        .filter(|skill_ref| !skill_ref.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    skill_refs.sort();
    skill_refs.dedup();
    skill_refs
}

fn skill_ref_input_is_valid(value: &str) -> bool {
    value.chars().count() <= 512
        && value
            .split(',')
            .map(str::trim)
            .filter(|skill_ref| !skill_ref.is_empty())
            .all(agent_profile_id_is_valid)
}

fn agent_profile_id_is_valid(value: &str) -> bool {
    project_id_is_valid(value)
}

fn project_id_is_valid(value: &str) -> bool {
    let length = value.chars().count();
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    let last = value.chars().next_back().unwrap_or(first);
    length <= 64
        && first.is_ascii_lowercase()
        && last.is_ascii_alphanumeric()
        && value.chars().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '-' | '_')
        })
}

fn project_name_is_valid(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty() && trimmed == value && trimmed.chars().count() <= 256
}

fn parse_project_execution_cap(value: &str) -> Option<u32> {
    value.trim().parse::<u32>().ok().filter(|value| *value > 0)
}

fn work_item_id_is_valid(value: &str) -> bool {
    let length = value.chars().count();
    !value.trim().is_empty()
        && value.trim() == value
        && length <= 128
        && !value.chars().any(char::is_control)
}

fn work_item_title_is_valid(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty() && trimmed == value && trimmed.chars().count() <= 256
}

fn parse_work_item_priority(value: &str) -> Option<u32> {
    value.trim().parse::<u32>().ok().filter(|value| *value > 0)
}

fn agent_role_is_valid(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty() && trimmed.chars().count() <= 128
}

fn agent_capability_catalog(
    agent_profiles: &[AgentProfileSummary],
    work_items: &[WorkItemSummary],
) -> Vec<String> {
    let mut capabilities = agent_profiles
        .iter()
        .flat_map(|profile| profile.capabilities.iter().cloned())
        .chain(
            work_items
                .iter()
                .flat_map(|item| item.required_capabilities.iter().cloned()),
        )
        .collect::<Vec<_>>();
    capabilities.sort();
    capabilities.dedup();
    capabilities
}

fn filter_work_items_by_project(
    work_items: &[WorkItemSummary],
    project_id: &str,
) -> Vec<WorkItemSummary> {
    work_items
        .iter()
        .filter(|work_item| project_id.is_empty() || work_item.project_id == project_id)
        .cloned()
        .collect()
}

fn work_item_drop_transition(
    item: &WorkItemSummary,
    target_state: WorkItemState,
) -> Option<WorkItemTransitionRequest> {
    (item.state != target_state).then(|| WorkItemTransitionRequest {
        project_id: item.project_id.clone(),
        work_item_id: item.id.clone(),
        expected_state: item.state,
        target_state,
    })
}

#[component]
fn PointerDragGhost(
    position: CanvasPoint,
    label: String,
    detail: String,
    variant: String,
) -> Element {
    let class_name = format!("pointer-drag-ghost {variant}");
    let style = format!("left: {:.1}px; top: {:.1}px;", position.x, position.y);
    rsx! {
        div {
            class: "{class_name}",
            style: "{style}",
            aria_hidden: "true",
            span { class: "pointer-drag-ghost-grip" }
            strong { "{label}" }
            small { "{detail}" }
        }
    }
}

fn pointer_drag_exceeded(origin: CanvasPoint, current: CanvasPoint) -> bool {
    const DRAG_THRESHOLD_PX: f64 = 6.0;
    let delta_x = current.x - origin.x;
    let delta_y = current.y - origin.y;
    delta_x.mul_add(delta_x, delta_y * delta_y) >= DRAG_THRESHOLD_PX * DRAG_THRESHOLD_PX
}

fn workspace_canvas_layout_id(prefix: &str, stable_id: &str) -> String {
    const MAX_IDENTIFIER_LEN: usize = 64;
    let raw = format!("{prefix}-{stable_id}");
    if raw.len() <= MAX_IDENTIFIER_LEN {
        return raw;
    }

    let hash = raw.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    });
    let suffix = format!("-{hash:016x}");
    let mut stem = raw[..MAX_IDENTIFIER_LEN - suffix.len()].to_owned();
    while stem.ends_with('-') || stem.ends_with('_') {
        stem.pop();
    }
    format!("{stem}{suffix}")
}

fn group_work_items_by_state(
    work_items: Vec<WorkItemSummary>,
) -> Vec<(WorkItemState, Vec<WorkItemSummary>)> {
    let mut grouped: [Vec<WorkItemSummary>; WORK_ITEM_STATES.len()] =
        std::array::from_fn(|_| Vec::new());
    for item in work_items {
        grouped[work_item_state_index(item.state)].push(item);
    }

    WORK_ITEM_STATES.into_iter().zip(grouped).collect()
}

fn work_item_state_index(state: WorkItemState) -> usize {
    match state {
        WorkItemState::Backlog => 0,
        WorkItemState::Todo => 1,
        WorkItemState::InProgress => 2,
        WorkItemState::InReview => 3,
        WorkItemState::Blocked => 4,
        WorkItemState::Done => 5,
        WorkItemState::Cancelled => 6,
    }
}

fn work_item_state_label(state: WorkItemState) -> &'static str {
    match state {
        WorkItemState::Backlog => "Backlog",
        WorkItemState::Todo => "Todo",
        WorkItemState::InProgress => "In progress",
        WorkItemState::InReview => "In review",
        WorkItemState::Blocked => "Blocked",
        WorkItemState::Done => "Done",
        WorkItemState::Cancelled => "Cancelled",
    }
}

fn kanban_lane_class(state: WorkItemState) -> &'static str {
    match state {
        WorkItemState::Backlog => "kanban-lane lane-backlog",
        WorkItemState::Todo => "kanban-lane lane-todo",
        WorkItemState::InProgress => "kanban-lane lane-running",
        WorkItemState::InReview => "kanban-lane lane-review",
        WorkItemState::Blocked => "kanban-lane lane-blocked",
        WorkItemState::Done => "kanban-lane lane-done",
        WorkItemState::Cancelled => "kanban-lane lane-cancelled",
    }
}

fn work_item_state_class(state: WorkItemState) -> &'static str {
    match state {
        WorkItemState::Backlog => "work-item-state state-backlog",
        WorkItemState::Todo => "work-item-state state-todo",
        WorkItemState::InProgress => "work-item-state state-running",
        WorkItemState::InReview => "work-item-state state-review",
        WorkItemState::Blocked => "work-item-state state-blocked",
        WorkItemState::Done => "work-item-state state-done",
        WorkItemState::Cancelled => "work-item-state state-cancelled",
    }
}

fn work_item_eligibility_label(state: WorkItemState) -> &'static str {
    match state {
        WorkItemState::Todo | WorkItemState::InProgress | WorkItemState::InReview => {
            "Eligible for active-work association"
        }
        WorkItemState::Backlog => "Not admitted to the executable queue",
        WorkItemState::Blocked => "Waiting for an explicit unblock",
        WorkItemState::Done | WorkItemState::Cancelled => "Terminal Work item",
    }
}

fn reconciliation_message(receipt: &ReconciliationReceipt) -> String {
    let reconciliation = &receipt.reconciliation;
    match reconciliation.decision {
        ReconciliationDecision::Dismissed => format!(
            "Suggestion dismissed. Work item remains {}.",
            reconciliation.resulting_state
        ),
        ReconciliationDecision::Accepted
            if reconciliation.previous_state == reconciliation.resulting_state =>
        {
            format!(
                "Suggestion accepted. Work item was already {}.",
                reconciliation.resulting_state
            )
        }
        ReconciliationDecision::Accepted => format!(
            "Suggestion accepted. Work item moved from {} to {}.",
            reconciliation.previous_state, reconciliation.resulting_state
        ),
    }
}

fn attachment_message(receipt: &AttachmentReceipt) -> String {
    if receipt.duplicate {
        format!(
            "Checkpoint was already attached to {}.",
            receipt.attachment.work_item_id
        )
    } else if receipt.created_work_item {
        format!(
            "Created {} in todo, attached the Checkpoint, and moved it out of the Activity Inbox.",
            receipt.attachment.work_item_id
        )
    } else {
        format!(
            "Checkpoint attached to {} and moved out of the Activity Inbox.",
            receipt.attachment.work_item_id
        )
    }
}

fn transition_message(receipt: &WorkItemTransitionReceipt) -> String {
    if receipt.changed {
        format!(
            "{} moved from {} to {}.",
            receipt.work_item_id, receipt.previous_state, receipt.resulting_state
        )
    } else {
        format!(
            "{} was already {}.",
            receipt.work_item_id, receipt.resulting_state
        )
    }
}

fn agent_plan_message(receipt: &AgentPlanUpdateReceipt) -> String {
    if !receipt.changed {
        return format!("{} already had this Agent plan.", receipt.work_item_id);
    }
    let agent = receipt
        .resulting
        .agent_profile_id
        .as_deref()
        .unwrap_or("Unassigned");
    let capability_count = receipt.resulting.required_capabilities.len();
    let capability_noun = if capability_count == 1 {
        "capability"
    } else {
        "capabilities"
    };
    format!(
        "{} Agent plan saved: {} with {} required {}.",
        receipt.work_item_id, agent, capability_count, capability_noun
    )
}

fn agent_profile_message(receipt: &AgentProfileSaveReceipt) -> String {
    if !receipt.changed {
        return format!(
            "Agent profile {} already had these settings.",
            receipt.resulting.id
        );
    }
    let action = if receipt.previous.is_none() {
        "created"
    } else {
        "saved"
    };
    let capability_count = receipt.resulting.capabilities.len();
    let capability_noun = if capability_count == 1 {
        "capability"
    } else {
        "capabilities"
    };
    format!(
        "Agent profile {} {action} with {capability_count} {capability_noun}.",
        receipt.resulting.id
    )
}

fn execution_workspace_message(receipt: &ExecutionWorkspaceSaveReceipt) -> String {
    if !receipt.changed {
        return format!(
            "{} was already connected to this Execution workspace.",
            receipt.resulting.project_id
        );
    }
    format!(
        "{} now uses {} for behavior reference checks.",
        receipt.resulting.project_id,
        execution_workspace_label(Some(&receipt.resulting))
    )
}

fn project_message(receipt: &ProjectCreateReceipt) -> String {
    format!(
        "Project {} added with {}. It is idle until you add Work items.",
        receipt.project.id,
        execution_workspace_label(Some(&receipt.execution_workspace))
    )
}

fn work_item_creation_message(receipt: &WorkItemCreateReceipt) -> String {
    format!(
        "Work item {} added to {} in todo.",
        receipt.work_item.id, receipt.work_item.project_id
    )
}

fn control_node_transition_message(receipt: &ControlNodeTransitionReceipt) -> String {
    format!(
        "{} advanced from {} to {} through {} with recorded evidence.",
        receipt.route.decision.work_item_id,
        receipt.route.decision.source_node_id,
        receipt.route.decision.next_node_id,
        receipt.route.decision.route_id
    )
}

fn next_control_decision_id() -> String {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("control-{}-{}", std::process::id(), elapsed.as_millis())
}

fn next_blueprint_application_id() -> String {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!(
        "blueprint-application-{}-{}",
        std::process::id(),
        elapsed.as_millis()
    )
}

fn next_graph_rewrite_ids() -> (String, String) {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let suffix = format!("{}-{}", std::process::id(), elapsed.as_millis());
    (format!("rewrite-{suffix}"), format!("candidate-{suffix}"))
}

fn next_graph_route_id(
    source_node_id: &str,
    destination_node_id: &str,
    signal: ControlSignal,
) -> String {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!(
        "route-{source_node_id}-{destination_node_id}-{}-{}",
        control_signal_value(signal),
        elapsed.as_millis()
    )
}

fn connect_graph_draft(
    draft: &mut Signal<Option<GraphDraft>>,
    source_node_id: String,
    destination_node_id: String,
    signal: ControlSignal,
) -> Result<(), String> {
    let route_id = next_graph_route_id(&source_node_id, &destination_node_id, signal);
    draft.write().as_mut().map_or_else(
        || Err("Graph draft is unavailable".to_owned()),
        |draft| {
            draft
                .connect(route_id, source_node_id, destination_node_id, signal)
                .map_err(|error| error.to_string())
        },
    )
}

fn execution_workspace_label(connection: Option<&ExecutionWorkspaceConnection>) -> String {
    match connection {
        Some(ExecutionWorkspaceConnection {
            kind: ExecutionWorkspaceKind::BundledSample,
            ..
        }) => "Bundled sample".to_owned(),
        Some(ExecutionWorkspaceConnection {
            kind: ExecutionWorkspaceKind::LocalDirectory,
            location: Some(location),
            ..
        }) => format!("Local directory · {location}"),
        _ => "Not connected".to_owned(),
    }
}

fn workspace_availability_label(availability: WorkspaceAvailability) -> &'static str {
    match availability {
        WorkspaceAvailability::BundledSample => "Bundled fixture",
        WorkspaceAvailability::Available => "Available",
        WorkspaceAvailability::Unavailable => "Unavailable",
    }
}

fn workspace_availability_class(availability: WorkspaceAvailability) -> &'static str {
    match availability {
        WorkspaceAvailability::BundledSample => "workspace-state workspace-state-sample",
        WorkspaceAvailability::Available => "workspace-state workspace-state-available",
        WorkspaceAvailability::Unavailable => "workspace-state workspace-state-unavailable",
    }
}

fn workspace_repository_label(inspection: &ExecutionWorkspaceInspection) -> String {
    if let Some(root) = &inspection.repository_root {
        return format!("Git root · {root}");
    }
    match inspection.availability {
        WorkspaceAvailability::BundledSample => "Bundled fixture".to_owned(),
        WorkspaceAvailability::Available => "No Git repository detected".to_owned(),
        WorkspaceAvailability::Unavailable => "Not checked while unavailable".to_owned(),
    }
}

fn workspace_instruction_discovery_note(
    availability: WorkspaceAvailability,
    has_no_instructions: bool,
) -> &'static str {
    if !has_no_instructions {
        return "";
    }
    match availability {
        WorkspaceAvailability::BundledSample => {
            "Bundled sample instructions are inspected from the Agent catalog."
        }
        WorkspaceAvailability::Available => "No root AGENTS.md was detected.",
        WorkspaceAvailability::Unavailable => {
            "Instruction discovery is unavailable until the directory returns."
        }
    }
}

fn workspace_skill_discovery_note(
    availability: WorkspaceAvailability,
    has_no_skills: bool,
) -> &'static str {
    if !has_no_skills {
        return "";
    }
    match availability {
        WorkspaceAvailability::BundledSample => {
            "Bundled sample Skills are inspected from the Agent catalog."
        }
        WorkspaceAvailability::Available => "No project-local Skills were detected.",
        WorkspaceAvailability::Unavailable => {
            "Skill discovery is unavailable until the directory returns."
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_form_requires_a_stable_identity_trimmed_name_and_positive_capacity() {
        assert!(project_id_is_valid("existing-product"));
        assert!(!project_id_is_valid("Existing product"));
        assert!(project_name_is_valid("Existing product"));
        assert!(!project_name_is_valid(" Existing product"));
        assert_eq!(parse_project_execution_cap("2"), Some(2));
        assert_eq!(parse_project_execution_cap("0"), None);
        assert_eq!(parse_project_execution_cap("not-a-number"), None);
    }

    #[test]
    fn work_item_form_requires_trimmed_identity_title_and_positive_priority() {
        assert!(work_item_id_is_valid("PROJECT-1"));
        assert!(!work_item_id_is_valid(" PROJECT-1"));
        assert!(work_item_title_is_valid("Describe the next outcome"));
        assert!(!work_item_title_is_valid("Describe the next outcome "));
        assert_eq!(parse_work_item_priority("4"), Some(4));
        assert_eq!(parse_work_item_priority("0"), None);
    }

    #[test]
    fn work_item_id_suggestion_continues_the_project_prefix() {
        let candidates = vec![
            WorkItemSummary {
                id: "CORE-1".to_owned(),
                project_id: "gareji-core".to_owned(),
                title: "First".to_owned(),
                priority: 1,
                state: WorkItemState::Done,
                approval_requirement: ApprovalRequirement::None,
                dependency_ids: Vec::new(),
                agent_profile_id: Some("implementer".to_owned()),
                required_capabilities: vec!["implementation".to_owned()],
            },
            WorkItemSummary {
                id: "CORE-4".to_owned(),
                project_id: "gareji-core".to_owned(),
                title: "Fourth".to_owned(),
                priority: 2,
                state: WorkItemState::Todo,
                approval_requirement: ApprovalRequirement::None,
                dependency_ids: Vec::new(),
                agent_profile_id: Some("implementer".to_owned()),
                required_capabilities: vec!["implementation".to_owned()],
            },
        ];

        assert_eq!(suggest_work_item_id("gareji-core", &candidates), "CORE-5");
        assert_eq!(suggest_work_item_id("new-project", &[]), "PROJECT-1");
    }

    #[test]
    fn agent_capability_catalog_is_stable_and_unique() {
        let profiles = vec![
            AgentProfileSummary {
                id: "reviewer".to_owned(),
                role: "Reviewer".to_owned(),
                capabilities: vec!["testing".to_owned(), "review".to_owned()],
                instruction_ref: None,
                skill_refs: Vec::new(),
            },
            AgentProfileSummary {
                id: "implementer".to_owned(),
                role: "Implementer".to_owned(),
                capabilities: vec!["implementation".to_owned(), "testing".to_owned()],
                instruction_ref: None,
                skill_refs: Vec::new(),
            },
        ];

        let work_items = vec![WorkItemSummary {
            id: "BOARD-9".to_owned(),
            project_id: "gareji-board".to_owned(),
            title: "Retire a legacy requirement".to_owned(),
            state: WorkItemState::Todo,
            priority: 1,
            approval_requirement: ApprovalRequirement::None,
            dependency_ids: Vec::new(),
            agent_profile_id: None,
            required_capabilities: vec!["legacy-capability".to_owned()],
        }];

        assert_eq!(
            agent_capability_catalog(&profiles, &work_items),
            vec![
                "implementation".to_owned(),
                "legacy-capability".to_owned(),
                "review".to_owned(),
                "testing".to_owned(),
            ]
        );
    }

    #[test]
    fn agent_profile_form_normalizes_capabilities_and_checks_stable_ids() {
        assert_eq!(
            parse_agent_capabilities(" testing, review, testing,  evidence "),
            vec![
                "evidence".to_owned(),
                "review".to_owned(),
                "testing".to_owned(),
            ]
        );
        assert!(agent_profile_id_is_valid("qa-specialist_2"));
        assert!(!agent_profile_id_is_valid("QA specialist"));
        assert!(!agent_profile_id_is_valid("-reviewer"));
        assert!(agent_role_is_valid("Quality reviewer"));
        assert!(!agent_role_is_valid("   "));
        assert_eq!(
            parse_agent_instruction_ref(" agents/qa/AGENT.md "),
            Some("agents/qa/AGENT.md".to_owned())
        );
        assert!(instruction_ref_input_is_valid("agents/qa/AGENT.md"));
        assert!(!instruction_ref_input_is_valid("../AGENT.md"));
        assert!(!instruction_ref_input_is_valid("C:/agents/AGENT.md"));
        assert_eq!(
            parse_agent_skill_refs(" review-work-item, evidence-summary, review-work-item "),
            vec!["evidence-summary".to_owned(), "review-work-item".to_owned()]
        );
        assert!(skill_ref_input_is_valid(
            "review-work-item, evidence-summary"
        ));
        assert!(!skill_ref_input_is_valid("Remote Skill"));
    }

    #[test]
    fn work_items_are_grouped_into_canonical_kanban_order() {
        let lanes = group_work_items_by_state(vec![
            WorkItemSummary {
                id: "BOARD-2".to_owned(),
                project_id: "gareji-board".to_owned(),
                title: "Second".to_owned(),
                priority: 2,
                state: WorkItemState::Blocked,
                approval_requirement: ApprovalRequirement::Explicit,
                dependency_ids: vec!["BOARD-1".to_owned()],
                agent_profile_id: Some("reviewer".to_owned()),
                required_capabilities: vec!["review".to_owned()],
            },
            WorkItemSummary {
                id: "BOARD-1".to_owned(),
                project_id: "gareji-board".to_owned(),
                title: "First".to_owned(),
                priority: 1,
                state: WorkItemState::Todo,
                approval_requirement: ApprovalRequirement::None,
                dependency_ids: Vec::new(),
                agent_profile_id: Some("implementer".to_owned()),
                required_capabilities: vec!["implementation".to_owned()],
            },
        ]);

        assert_eq!(lanes.len(), WORK_ITEM_STATES.len());
        assert_eq!(
            lanes.iter().map(|(state, _)| *state).collect::<Vec<_>>(),
            WORK_ITEM_STATES
        );
        assert_eq!(
            lanes[work_item_state_index(WorkItemState::Todo)].1[0].id,
            "BOARD-1"
        );
        assert_eq!(
            lanes[work_item_state_index(WorkItemState::Blocked)].1[0].id,
            "BOARD-2"
        );
        assert_eq!(lanes.iter().map(|(_, items)| items.len()).sum::<usize>(), 2);
    }

    #[test]
    fn kanban_filter_keeps_only_the_selected_project_without_mutating_items() {
        let work_items = vec![
            WorkItemSummary {
                id: "BOARD-1".to_owned(),
                project_id: "gareji-board".to_owned(),
                title: "Board work".to_owned(),
                priority: 1,
                state: WorkItemState::Todo,
                approval_requirement: ApprovalRequirement::None,
                dependency_ids: Vec::new(),
                agent_profile_id: None,
                required_capabilities: Vec::new(),
            },
            WorkItemSummary {
                id: "CORE-1".to_owned(),
                project_id: "gareji-core".to_owned(),
                title: "Core work".to_owned(),
                priority: 1,
                state: WorkItemState::Todo,
                approval_requirement: ApprovalRequirement::None,
                dependency_ids: Vec::new(),
                agent_profile_id: None,
                required_capabilities: Vec::new(),
            },
        ];

        assert_eq!(filter_work_items_by_project(&work_items, "").len(), 2);
        let filtered = filter_work_items_by_project(&work_items, "gareji-core");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "CORE-1");
        assert_eq!(work_items.len(), 2);
    }

    #[test]
    fn kanban_card_drop_builds_a_transition_only_for_a_different_lane() {
        let item = WorkItemSummary {
            id: "BOARD-1".to_owned(),
            project_id: "gareji-board".to_owned(),
            title: "Make the Board easier to scan".to_owned(),
            priority: 1,
            state: WorkItemState::Todo,
            approval_requirement: ApprovalRequirement::None,
            dependency_ids: Vec::new(),
            agent_profile_id: Some("implementer".to_owned()),
            required_capabilities: vec!["implementation".to_owned()],
        };

        assert_eq!(
            work_item_drop_transition(&item, WorkItemState::InProgress),
            Some(WorkItemTransitionRequest {
                project_id: "gareji-board".to_owned(),
                work_item_id: "BOARD-1".to_owned(),
                expected_state: WorkItemState::Todo,
                target_state: WorkItemState::InProgress,
            })
        );
        assert_eq!(work_item_drop_transition(&item, WorkItemState::Todo), None);
    }

    #[test]
    fn pointer_drag_waits_for_a_deliberate_movement_threshold() {
        let origin = CanvasPoint { x: 100.0, y: 200.0 };

        assert!(!pointer_drag_exceeded(
            origin,
            CanvasPoint { x: 104.0, y: 203.0 },
        ));
        assert!(pointer_drag_exceeded(
            origin,
            CanvasPoint { x: 108.0, y: 206.0 },
        ));
    }

    #[test]
    fn graph_drag_keeps_the_node_offset_instead_of_jumping_to_the_stage_origin() {
        let start_position = CanvasPoint { x: 640.0, y: 320.0 };
        let pointer_origin = CanvasPoint { x: 722.0, y: 362.0 };
        let pointer_current = CanvasPoint { x: 746.0, y: 380.0 };

        assert_eq!(
            graph_drag_preview_point(start_position, pointer_origin, pointer_current, 1.0),
            CanvasPoint { x: 664.0, y: 338.0 },
        );
        assert_eq!(
            graph_drag_preview_point(start_position, pointer_origin, pointer_current, 0.8),
            CanvasPoint { x: 670.0, y: 342.5 },
        );
    }

    #[test]
    fn visual_branch_pins_keep_primary_existing_and_selected_signals_unique() {
        let options = branch_signal_options(
            &STANDARD_GRAPH_SIGNALS,
            &AGENT_LOOP_PRIMARY_BRANCH_SIGNALS,
            &[ControlSignal::Succeeded, ControlSignal::BudgetExceeded],
            Some(ControlSignal::Manual),
        );

        assert_eq!(
            options,
            vec![
                ControlSignal::Succeeded,
                ControlSignal::Failed,
                ControlSignal::BudgetExceeded,
                ControlSignal::Manual,
            ]
        );
        assert!(
            (graph_branch_port_y(&options, &ControlSignal::Succeeded) - GRAPH_BRANCH_PORT_START_Y)
                .abs()
                < f64::EPSILON
        );
        assert!(
            (graph_branch_port_y(&options, &ControlSignal::Failed)
                - (GRAPH_BRANCH_PORT_START_Y + GRAPH_BRANCH_PORT_STEP_Y))
                .abs()
                < f64::EPSILON
        );
        assert_eq!(
            branch_signal_options(
                &APPROVAL_GRAPH_SIGNALS,
                &APPROVAL_GRAPH_SIGNALS,
                &[ControlSignal::Approved],
                Some(ControlSignal::Succeeded),
            ),
            APPROVAL_GRAPH_SIGNALS,
        );
    }

    #[test]
    fn blank_branch_drop_keeps_the_connection_until_a_node_is_created() {
        assert_eq!(
            pending_control_branch_spawn(
                "audit",
                Some(ControlSignal::Failed),
                CanvasPoint { x: 620.0, y: 280.0 },
            ),
            Some(PendingControlBranchSpawn {
                source_node_id: "audit".to_owned(),
                signal: ControlSignal::Failed,
                point: CanvasPoint { x: 620.0, y: 280.0 },
            })
        );
        assert_eq!(
            pending_control_branch_spawn("", Some(ControlSignal::Failed), CanvasPoint::default(),),
            None
        );
    }

    #[test]
    fn large_desktop_layout_expands_the_workbench_without_css_zoom() {
        assert!(APP_CSS.contains("width: min(2160px, calc(100% - clamp(48px, 4vw, 96px)))"));
        assert!(APP_CSS.contains("@media (min-width: 1500px) and (min-height: 800px)"));
        assert!(APP_CSS.contains("width: min(1720px"));
        assert!(APP_CSS.contains("@media (min-width: 1900px) and (min-height: 1000px)"));
        assert!(APP_CSS.contains("width: min(2160px"));
        assert!(APP_CSS.contains(".graph-stage-scale, .graph-freeform-stage { min-width: 100%; }"));
        assert!(!APP_CSS.contains(".shell { zoom:"));
    }

    #[test]
    fn focused_graph_workspaces_keep_the_canvas_dominant() {
        assert!(APP_CSS.contains(".workspace-page-body"));
        assert!(APP_CSS.contains("grid-template-rows: auto auto auto minmax(0, 1fr) auto;"));
        assert!(APP_CSS.contains(
            ".workspace-page-body .portfolio-orchestration-canvas > .graph-layout-scroll"
        ));
        assert!(!APP_CSS.contains("workspace-detail-modal"));
        assert!(!APP_CSS.contains("catalog-detail-modal"));
    }

    #[test]
    fn workspace_canvas_layout_identity_stays_bounded_and_deterministic() {
        let stable_id = "a".repeat(64);
        let first = workspace_canvas_layout_id("blueprint", &stable_id);
        let second = workspace_canvas_layout_id("blueprint", &stable_id);

        assert_eq!(first, second);
        assert!(first.len() <= 64);
        assert!(first.starts_with("blueprint-"));
        assert!(first.chars().all(|character| character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || matches!(character, '-' | '_')));
    }

    #[test]
    fn graph_manager_keeps_a_valid_project_and_falls_back_to_the_first_managed_one() {
        let project_ids = vec!["gareji-board".to_owned(), "gareji-core".to_owned()];

        assert_eq!(
            resolve_managed_project_id(&project_ids, "gareji-core"),
            Some("gareji-core")
        );
        assert_eq!(
            resolve_managed_project_id(&project_ids, "removed-project"),
            Some("gareji-board")
        );
        assert_eq!(resolve_managed_project_id(&[], "gareji-core"), None);
    }
}

#[allow(clippy::too_many_lines)]
#[component]
fn BlueprintStudioPanel(
    blueprints: Vec<OrchestrationBlueprintRevision>,
    graph_canvas_layouts: Vec<GraphCanvasLayout>,
    approach_notes: Vec<ApproachNoteManifest>,
    note_warning: Option<String>,
    preview: Option<BlueprintApplicationPreview>,
    applications: Vec<BlueprintApplicationReceipt>,
    application_notice: Option<String>,
    revision_notice: Option<String>,
    on_preview: EventHandler<MouseEvent>,
    on_accept: EventHandler<BlueprintApplicationProposal>,
    on_save_revision: EventHandler<OrchestrationBlueprintRevision>,
    on_graph_canvas_layout: EventHandler<GraphCanvasLayout>,
) -> Element {
    let initial_revision = blueprints.first().map_or_else(String::new, |blueprint| {
        format!("{}|{}", blueprint.blueprint_id, blueprint.revision_id)
    });
    let initial_blueprint = blueprints.first().cloned();
    let mut selected_revision = use_signal(move || initial_revision);
    let mut draft = use_signal(move || {
        initial_blueprint.and_then(|blueprint| BlueprintDraft::new(blueprint).ok())
    });
    let initial_node_id = blueprints
        .first()
        .map_or_else(String::new, |blueprint| blueprint.entry_node_id.clone());
    let mut selected_node_id = use_signal(move || initial_node_id);
    let mut dragged_note_id = use_signal(String::new);
    let mut connection_target_id = use_signal(String::new);
    let mut connection_kind = use_signal(|| "flow".to_owned());
    let mut connection_signal = use_signal(|| "succeeded".to_owned());
    let mut connection_socket = use_signal(|| "evidence".to_owned());
    let mut stage_kind = use_signal(|| "gate".to_owned());
    let mut candidate_revision_id = use_signal(String::new);
    let mut editor_notice = use_signal(String::new);
    let selected_value = selected_revision.read().clone();
    let active_node_id = selected_node_id.read().clone();
    let target_value = connection_target_id.read().clone();
    let connection_kind_value = connection_kind.read().clone();
    let connection_signal_value = connection_signal.read().clone();
    let connection_socket_value = connection_socket.read().clone();
    let stage_kind_value = stage_kind.read().clone();
    let candidate_revision_value = candidate_revision_id.read().clone();
    let editor_notice_value = editor_notice.read().clone();
    let draft_blueprint = draft.read().as_ref().map(|draft| draft.blueprint().clone());
    let blueprint_canvas_layout_id = draft_blueprint
        .as_ref()
        .map_or_else(String::new, |blueprint| {
            workspace_canvas_layout_id("blueprint", &blueprint.blueprint_id)
        });
    let blueprint_canvas_layout = graph_canvas_layouts
        .iter()
        .find(|layout| layout.graph_id == blueprint_canvas_layout_id)
        .cloned();
    let selected_node = draft_blueprint.as_ref().and_then(|blueprint| {
        blueprint
            .nodes
            .iter()
            .find(|node| node.id == active_node_id)
            .cloned()
    });
    let allowed_flow_signals =
        blueprint_signals_for_node(selected_node.as_ref().map(|node| &node.kind));
    let selected_flow_signal = parse_control_signal(&connection_signal_value)
        .filter(|signal| allowed_flow_signals.contains(signal))
        .or_else(|| allowed_flow_signals.first().copied())
        .unwrap_or(ControlSignal::Manual);
    let selected_data_socket = parse_note_socket(&connection_socket_value)
        .filter(|socket| {
            selected_node
                .as_ref()
                .is_some_and(|node| node.outputs.contains(socket))
        })
        .or_else(|| {
            selected_node
                .as_ref()
                .and_then(|node| node.outputs.first().copied())
        });
    let selected_data_socket_value = selected_data_socket.map_or("", note_socket_value);
    let has_changes = draft
        .read()
        .as_ref()
        .is_some_and(BlueprintDraft::has_changes);
    let can_undo = draft.read().as_ref().is_some_and(BlueprintDraft::can_undo);
    let can_save = !candidate_revision_value.trim().is_empty()
        && draft
            .read()
            .as_ref()
            .is_some_and(|draft| draft.revision(candidate_revision_value.trim()).is_ok());
    let revision_options = blueprints.clone();
    let drop_notes = approach_notes.clone();

    rsx! {
        section { class: "section-heading blueprint-studio-heading",
            div {
                p { class: "kicker", "Reusable orchestration" }
                h3 { "Blueprint Studio" }
            }
            span { "Project-independent" }
        }
        section { class: "blueprint-studio", aria_label: "Orchestration Blueprint Studio",
            header { class: "blueprint-studio-header",
                div {
                    strong { "Build the approach once. Preview where it fits." }
                    p { "Projects, Agent profiles, and workspaces are resolved only in a proposed application." }
                }
                button { onclick: move |event| on_preview.call(event), "Preview applications" }
            }
            if let Some(warning) = &note_warning {
                p { class: "blueprint-note-warning", "{warning}" }
            }
            if let Some(notice) = &application_notice {
                p { class: "blueprint-application-notice", "{notice}" }
            }
            if let Some(notice) = &revision_notice {
                p { class: "blueprint-application-notice", "{notice}" }
            }
            if let Some(blueprint) = draft_blueprint.clone() {
                if blueprints.len() > 1 {
                    label { class: "blueprint-revision-select",
                        span { "Blueprint revision" }
                        select {
                            aria_label: "Orchestration Blueprint revision",
                            value: "{selected_value}",
                            onchange: move |event| {
                                let value = event.value();
                                if let Some(source) = revision_options.iter().find(|revision| {
                                    format!("{}|{}", revision.blueprint_id, revision.revision_id) == value
                                }) {
                                    selected_revision.set(value);
                                    selected_node_id.set(source.entry_node_id.clone());
                                    draft.set(BlueprintDraft::new(source.clone()).ok());
                                    editor_notice.set(String::new());
                                    candidate_revision_id.set(String::new());
                                }
                            },
                            for option in &blueprints {
                                option {
                                    value: "{option.blueprint_id}|{option.revision_id}",
                                    "{option.name} · {option.revision_id}"
                                }
                            }
                        }
                    }
                }
                div { class: "blueprint-studio-grid",
                    aside { class: "blueprint-note-palette",
                        div { class: "blueprint-pane-title",
                            span { "Approach Notes" }
                            strong { "{approach_notes.len()}" }
                        }
                        if approach_notes.is_empty() {
                            p { class: "blueprint-pane-empty", "Add Markdown files under approaches/ or configure their absolute paths during setup." }
                        } else {
                            for note in &approach_notes {
                                article {
                                    class: "blueprint-note-card",
                                    key: "note-{note.approach_id}",
                                    draggable: true,
                                    ondragstart: {
                                        let approach_id = note.approach_id.clone();
                                        move |_| dragged_note_id.set(approach_id.clone())
                                    },
                                    div {
                                        strong { "{note.title}" }
                                        small { "{note.approach_id}" }
                                    }
                                    span { class: "blueprint-risk", "{approach_risk_label(note.risk)}" }
                                    div { class: "blueprint-socket-row",
                                        for socket in &note.inputs {
                                            span { class: "blueprint-socket input", "{note_socket_label(*socket)}" }
                                        }
                                        for socket in &note.outputs {
                                            span { class: "blueprint-socket output", "{note_socket_label(*socket)}" }
                                        }
                                    }
                                    small { class: "blueprint-note-path", title: "{note.absolute_path}", "{note.absolute_path}" }
                                    button {
                                        onclick: {
                                            let note = note.clone();
                                            move |_| match add_note_to_blueprint_draft(&mut draft, &note) {
                                                Ok(node_id) => {
                                                    selected_node_id.set(node_id);
                                                    editor_notice.set("Approach Note snapped into the draft.".to_owned());
                                                }
                                                Err(error) => editor_notice.set(error),
                                            }
                                        },
                                        "Add to graph"
                                    }
                                }
                            }
                        }
                    }
                    BlueprintCanvas {
                        key: "{blueprint_canvas_layout_id}",
                        blueprint: blueprint.clone(),
                        canvas_layout: blueprint_canvas_layout.clone(),
                        canvas_layout_id: blueprint_canvas_layout_id.clone(),
                        active_node_id: active_node_id.clone(),
                        flow_signal: selected_flow_signal,
                        can_undo,
                        can_reset: has_changes,
                        on_node_select: move |node_id| selected_node_id.set(node_id),
                        on_note_drop: move |()| {
                            let approach_id = dragged_note_id.read().clone();
                            let result = drop_notes
                                .iter()
                                .find(|note| note.approach_id == approach_id)
                                .ok_or_else(|| "Drag a loaded Approach Note onto the canvas.".to_owned())
                                .and_then(|note| add_note_to_blueprint_draft(&mut draft, note));
                            dragged_note_id.set(String::new());
                            match result {
                                Ok(node_id) => {
                                    selected_node_id.set(node_id);
                                    editor_notice.set("Approach Note snapped into the draft.".to_owned());
                                }
                                Err(error) => editor_notice.set(error),
                            }
                        },
                        on_flow_connect: move |(source, destination, signal): (String, String, ControlSignal)| {
                            match connect_blueprint_draft(
                                &mut draft,
                                source.clone(),
                                destination,
                                BlueprintLinkKind::Flow { signal },
                            ) {
                                Ok(()) => {
                                    selected_node_id.set(source);
                                    editor_notice.set("Flow connected by drag and drop.".to_owned());
                                }
                                Err(error) => editor_notice.set(error),
                            }
                        },
                        on_branch_spawn: move |(source, signal, node): (String, ControlSignal, BlueprintNode)| {
                            let node_id = node.id.clone();
                            let result = draft.write().as_mut().map_or_else(
                                || Err("Blueprint draft is unavailable".to_owned()),
                                |draft| {
                                    draft.add_node(node).map_err(|error| error.to_string())?;
                                    let link_id = next_blueprint_link_id();
                                    if let Err(error) = draft.connect(
                                        link_id,
                                        source,
                                        node_id.clone(),
                                        BlueprintLinkKind::Flow { signal },
                                    ) {
                                        let _ = draft.undo();
                                        return Err(error.to_string());
                                    }
                                    Ok(())
                                },
                            );
                            match result {
                                Ok(()) => {
                                    selected_node_id.set(node_id);
                                    editor_notice.set("Created and connected a new branch destination.".to_owned());
                                }
                                Err(error) => editor_notice.set(error),
                            }
                        },
                        on_remove_link: move |link_id: String| {
                            let result = draft.write().as_mut().map_or_else(
                                || Err("Blueprint draft is unavailable".to_owned()),
                                |draft| draft.remove_link(&link_id).map_err(|error| error.to_string()),
                            );
                            match result {
                                Ok(()) => editor_notice.set("Connection removed from the draft.".to_owned()),
                                Err(error) => editor_notice.set(error),
                            }
                        },
                        on_undo: move |()| {
                            let result = draft.write().as_mut().map_or_else(
                                || Err("Blueprint draft is unavailable".to_owned()),
                                |draft| draft.undo().map_err(|error| error.to_string()),
                            );
                            match result {
                                Ok(()) => editor_notice.set("Last Blueprint edit undone.".to_owned()),
                                Err(error) => editor_notice.set(error),
                            }
                        },
                        on_reset: move |()| {
                            let result = draft.write().as_mut().map_or_else(
                                || Err("Blueprint draft is unavailable".to_owned()),
                                |draft| draft.reset().map_err(|error| error.to_string()),
                            );
                            match result {
                                Ok(()) => {
                                    let entry = draft.read().as_ref().map_or_else(
                                        String::new,
                                        |draft| draft.blueprint().entry_node_id.clone(),
                                    );
                                    selected_node_id.set(entry);
                                    editor_notice.set("Blueprint draft reset to its saved revision.".to_owned());
                                }
                                Err(error) => editor_notice.set(error),
                            }
                        },
                        on_layout_save: on_graph_canvas_layout,
                    }
                    BlueprintInspector {
                        blueprint: blueprint.clone(),
                        node_id: active_node_id.clone(),
                        approach_notes: approach_notes.clone(),
                    }
                }
                div { class: "blueprint-editor-tools",
                    section { class: "graph-builder-tool",
                        h4 { "Add stage" }
                        select {
                            aria_label: "Blueprint stage type",
                            value: "{stage_kind_value}",
                            onchange: move |event| stage_kind.set(event.value()),
                            option { value: "gate", "Gate" }
                            option { value: "audit", "Audit" }
                            option { value: "approval", "Approval" }
                            option { value: "terminal", "Terminal" }
                        }
                        button {
                            onclick: move |_| {
                                let result = draft.write().as_mut().map_or_else(
                                    || Err("Blueprint draft is unavailable".to_owned()),
                                    |draft| {
                                        let node = blueprint_stage_node(draft.blueprint(), &stage_kind_value)
                                            .ok_or_else(|| "Choose a supported stage type.".to_owned())?;
                                        let node_id = node.id.clone();
                                        draft.add_node(node).map_err(|error| error.to_string())?;
                                        Ok(node_id)
                                    },
                                );
                                match result {
                                    Ok(node_id) => {
                                        selected_node_id.set(node_id);
                                        editor_notice.set("Stage added. Connect it before saving.".to_owned());
                                    }
                                    Err(error) => editor_notice.set(error),
                                }
                            },
                            "Add stage"
                        }
                    }
                    section { class: "graph-builder-tool",
                        h4 { "Connect selected node" }
                        p { class: "graph-builder-selection", "From: {active_node_id}" }
                        select {
                            aria_label: "Blueprint connection destination",
                            value: "{target_value}",
                            onchange: move |event| connection_target_id.set(event.value()),
                            option { value: "", "Select destination" }
                            for node in &blueprint.nodes {
                                option {
                                    value: "{node.id}",
                                    disabled: node.id == active_node_id,
                                    "{node.id}"
                                }
                            }
                        }
                        select {
                            aria_label: "Blueprint connection type",
                            value: "{connection_kind_value}",
                            onchange: move |event| connection_kind.set(event.value()),
                            option { value: "flow", "Flow" }
                            option { value: "data", "Typed data" }
                        }
                        if connection_kind_value == "flow" {
                            select {
                                aria_label: "Blueprint flow signal",
                                value: "{control_signal_value(selected_flow_signal)}",
                                onchange: move |event| connection_signal.set(event.value()),
                                for signal in allowed_flow_signals {
                                    option {
                                        value: "{control_signal_value(*signal)}",
                                        "{control_signal_label(*signal)}"
                                    }
                                }
                            }
                        } else {
                            select {
                                aria_label: "Blueprint data socket",
                                value: "{selected_data_socket_value}",
                                onchange: move |event| connection_socket.set(event.value()),
                                if let Some(node) = &selected_node {
                                    for socket in &node.outputs {
                                        option {
                                            value: "{note_socket_value(*socket)}",
                                            "{note_socket_label(*socket)}"
                                        }
                                    }
                                }
                            }
                        }
                        button {
                            disabled: active_node_id.is_empty()
                                || target_value.is_empty()
                                || (connection_kind_value == "flow" && allowed_flow_signals.is_empty())
                                || (connection_kind_value == "data" && selected_data_socket.is_none()),
                            onclick: move |_| {
                                let kind = if connection_kind_value == "flow" {
                                    Some(BlueprintLinkKind::Flow { signal: selected_flow_signal })
                                } else {
                                    selected_data_socket.map(|socket| BlueprintLinkKind::Data { socket })
                                };
                                let result = kind.ok_or_else(|| "Choose a compatible socket or flow.".to_owned())
                                    .and_then(|kind| connect_blueprint_draft(
                                        &mut draft,
                                        active_node_id.clone(),
                                        target_value.clone(),
                                        kind,
                                    ));
                                match result {
                                    Ok(()) => {
                                        connection_target_id.set(String::new());
                                        editor_notice.set("Connection added to the draft.".to_owned());
                                    }
                                    Err(error) => editor_notice.set(error),
                                }
                            },
                            "Connect"
                        }
                    }
                    section { class: "graph-builder-tool",
                        h4 { "Selected node" }
                        if let Some(node) = &selected_node {
                            strong { "{node.id}" }
                            small { "{blueprint_node_kind_label(&node.kind)}" }
                            button {
                                class: "graph-node-remove",
                                disabled: node.id == blueprint.entry_node_id,
                                onclick: {
                                    let node_id = node.id.clone();
                                    move |_| {
                                        let result = draft.write().as_mut().map_or_else(
                                            || Err("Blueprint draft is unavailable".to_owned()),
                                            |draft| draft.remove_node(&node_id).map_err(|error| error.to_string()),
                                        );
                                        match result {
                                            Ok(()) => {
                                                selected_node_id.set(String::new());
                                                editor_notice.set("Node and its connections removed.".to_owned());
                                            }
                                            Err(error) => editor_notice.set(error),
                                        }
                                    }
                                },
                                if node.id == blueprint.entry_node_id { "Entry node (fixed)" } else { "Remove node" }
                            }
                        }
                    }
                    section { class: "graph-builder-tool blueprint-save-tool",
                        h4 { "Save immutable revision" }
                        input {
                            aria_label: "New Blueprint revision ID",
                            value: "{candidate_revision_value}",
                            maxlength: 64,
                            placeholder: "v2",
                            oninput: move |event| candidate_revision_id.set(event.value()),
                        }
                        button {
                            disabled: !can_save,
                            onclick: move |_| {
                                if let Some(candidate) = draft.read().as_ref()
                                    .and_then(|draft| draft.revision(candidate_revision_value.trim()).ok())
                                {
                                    on_save_revision.call(candidate);
                                }
                            },
                            "Save new revision"
                        }
                        small { if has_changes && !can_save {
                            "Connect every stage and enter a new revision ID."
                        } else {
                            "Saving does not apply it to a Project or start a Runner."
                        } }
                    }
                }
                if !editor_notice_value.is_empty() {
                    p { class: "graph-editor-notice", "{editor_notice_value}" }
                }
                if let Some(preview) = &preview {
                    BlueprintApplicationPreviewPanel {
                        blueprint_id: blueprint.blueprint_id.clone(),
                        revision_id: blueprint.revision_id.clone(),
                        preview: preview.clone(),
                        applications: applications.clone(),
                        on_accept,
                    }
                } else {
                    p { class: "blueprint-preview-empty", "Preview shows which managed projects can accept this Blueprint without starting work." }
                }
            } else {
                p { class: "blueprint-pane-empty", "No Orchestration Blueprint revision is available." }
            }
        }
    }
}

#[component]
fn BlueprintCanvas(
    blueprint: OrchestrationBlueprintRevision,
    canvas_layout: Option<GraphCanvasLayout>,
    canvas_layout_id: String,
    active_node_id: String,
    flow_signal: ControlSignal,
    can_undo: bool,
    can_reset: bool,
    on_node_select: EventHandler<String>,
    on_note_drop: EventHandler<()>,
    on_flow_connect: EventHandler<(String, String, ControlSignal)>,
    on_branch_spawn: EventHandler<(String, ControlSignal, BlueprintNode)>,
    on_remove_link: EventHandler<String>,
    on_undo: EventHandler<()>,
    on_reset: EventHandler<()>,
    on_layout_save: EventHandler<GraphCanvasLayout>,
) -> Element {
    let initial_node_ids = blueprint
        .nodes
        .iter()
        .map(|node| node.id.clone())
        .collect::<Vec<_>>();
    let initial_positions = canvas_position_map(canvas_layout.as_ref(), &initial_node_ids);
    let mut dragged_node_id = use_signal(String::new);
    let mut dragged_route_source_id = use_signal(String::new);
    let mut dragged_route_signal = use_signal(|| None::<ControlSignal>);
    let mut drop_target_node_id = use_signal(String::new);
    let mut node_positions = use_signal(move || initial_positions);
    let mut drag_origin = use_signal(|| None::<CanvasPoint>);
    let mut drag_start_position = use_signal(|| None::<CanvasPoint>);
    let mut pointer_drag_active = use_signal(|| false);
    let mut drag_preview = use_signal(|| None::<(String, CanvasPoint)>);
    let mut branch_spawn_menu = use_signal(|| None::<PendingControlBranchSpawn>);
    let drop_target_value = drop_target_node_id.read().clone();
    let dragged_node_value = dragged_node_id.read().clone();
    let pointer_drag_value = *pointer_drag_active.read();
    let branch_spawn_value = branch_spawn_menu.read().clone();
    let branch_spawn_style = branch_spawn_value
        .as_ref()
        .map_or_else(String::new, |pending| {
            format!(
                "left: {:.1}px; top: {:.1}px;",
                pending.point.x, pending.point.y
            )
        });
    let node_ids = initial_node_ids;
    let flow_links = blueprint
        .links
        .iter()
        .filter(|link| matches!(link.kind, BlueprintLinkKind::Flow { .. }))
        .map(|link| {
            (
                link.source_node_id.clone(),
                link.destination_node_id.clone(),
            )
        })
        .collect::<Vec<_>>();
    let layout = ControlGraphLayout::from_topology(
        &node_ids,
        std::slice::from_ref(&blueprint.entry_node_id),
        &flow_links,
    );
    let mut rendered_positions = node_positions.read().clone();
    if let Some((node_id, point)) = drag_preview.read().clone() {
        rendered_positions.insert(node_id, point);
    }
    let (stage_width, stage_height) = layout.freeform_stage_dimensions(&rendered_positions);
    let stage_style = format!("width: {stage_width}px; min-height: {stage_height}px;");
    let view_box = format!("0 0 {stage_width} {stage_height}");
    let arrange_layout_id = canvas_layout_id.clone();
    let spawn_layout_id = canvas_layout_id.clone();
    let move_layout_id = canvas_layout_id;
    rsx! {
        section { class: "blueprint-canvas", aria_label: "Blueprint node canvas",
            header { class: "blueprint-pane-title",
                div {
                    span { "{blueprint.blueprint_id} · {blueprint.revision_id}" }
                    strong { "{blueprint.name}" }
                }
                div { class: "blueprint-canvas-actions",
                    button {
                        disabled: !can_undo,
                        onclick: move |_| on_undo.call(()),
                        "Undo"
                    }
                    button {
                        disabled: !can_reset,
                        onclick: move |_| on_reset.call(()),
                        "Reset"
                    }
                    button {
                        disabled: node_positions.read().is_empty(),
                        title: "Return moved nodes to automatic layout",
                        onclick: move |_| {
                            node_positions.write().clear();
                            on_layout_save.call(graph_canvas_layout_from_positions(
                                &arrange_layout_id,
                                &node_positions.read(),
                            ));
                        },
                        "Auto arrange"
                    }
                }
            }
            small { class: "blueprint-canvas-hint", "Drag cards to move them · drop a labeled branch pin on a node to connect, or on empty canvas to create its destination" }
            div { class: "graph-link-legend", aria_label: "Connection legend",
                span { class: "flow", "Solid green · control flow" }
                span { class: "data", "Purple dotted · typed note data" }
            }
            div {
                class: "graph-layout-scroll blueprint-canvas-scroll",
                ondragover: move |event| event.prevent_default(),
                ondrop: move |event| {
                    event.prevent_default();
                    on_note_drop.call(());
                },
                div {
                    class: "graph-stage graph-freeform-stage",
                    style: "{stage_style}",
                    onpointermove: move |event| {
                        let node_id = dragged_node_id.read().clone();
                        if node_id.is_empty() {
                            return;
                        }
                        let current = event.client_coordinates();
                        let current = CanvasPoint { x: current.x, y: current.y };
                        let origin = *drag_origin.read();
                        let exceeded = origin
                            .is_some_and(|origin| pointer_drag_exceeded(origin, current));
                        if exceeded || *pointer_drag_active.read() {
                            pointer_drag_active.set(true);
                            if let (Some(origin), Some(start_position)) =
                                (origin, *drag_start_position.read())
                            {
                                drag_preview.set(Some((
                                    node_id,
                                    graph_drag_preview_point(
                                        start_position,
                                        origin,
                                        current,
                                        1.0,
                                    ),
                                )));
                            }
                        }
                    },
                    onpointerup: move |event| {
                        let node_id = dragged_node_id.read().clone();
                        if !node_id.is_empty() && *pointer_drag_active.read() {
                            let final_point = drag_preview
                                .read()
                                .as_ref()
                                .filter(|(preview_node_id, _)| preview_node_id == &node_id)
                                .map(|(_, preview_point)| *preview_point)
                                .or(*drag_start_position.read());
                            if let Some(final_point) = final_point {
                                node_positions.write().insert(node_id, final_point);
                                on_layout_save.call(graph_canvas_layout_from_positions(
                                    &move_layout_id,
                                    &node_positions.read(),
                                ));
                            }
                        }
                        let source_node_id = dragged_route_source_id.read().clone();
                        if !source_node_id.is_empty() {
                            let coordinates = event.element_coordinates();
                            let point = graph_canvas_drop_point(coordinates.x, coordinates.y, 1.0);
                            branch_spawn_menu.set(pending_control_branch_spawn(
                                &source_node_id,
                                *dragged_route_signal.read(),
                                point,
                            ));
                        }
                        dragged_node_id.set(String::new());
                        dragged_route_source_id.set(String::new());
                        dragged_route_signal.set(None);
                        drop_target_node_id.set(String::new());
                        drag_origin.set(None);
                        drag_start_position.set(None);
                        pointer_drag_active.set(false);
                        drag_preview.set(None);
                    },
                    onpointercancel: move |_| {
                        dragged_node_id.set(String::new());
                        dragged_route_source_id.set(String::new());
                        dragged_route_signal.set(None);
                        drop_target_node_id.set(String::new());
                        drag_origin.set(None);
                        drag_start_position.set(None);
                        pointer_drag_active.set(false);
                        drag_preview.set(None);
                    },
                    svg {
                        class: "graph-route-layer",
                        view_box: "{view_box}",
                        width: "{stage_width}",
                        height: "{stage_height}",
                        defs {
                            marker {
                                id: "blueprint-flow-arrow",
                                marker_width: "7",
                                marker_height: "7",
                                ref_x: "6",
                                ref_y: "3",
                                orient: "auto",
                                marker_units: "strokeWidth",
                                path { d: "M 0 0 L 6 3 L 0 6 z" }
                            }
                        }
                        for link in &blueprint.links {
                            if let Some(path_data) = layout.freeform_branch_route_path(
                                &link.source_node_id,
                                &link.destination_node_id,
                                &rendered_positions,
                                match link.kind {
                                    BlueprintLinkKind::Flow { signal } => {
                                        blueprint_route_source_port_y(
                                            &blueprint,
                                            &link.source_node_id,
                                            signal,
                                            flow_signal,
                                        )
                                    }
                                    BlueprintLinkKind::Data { .. } => GRAPH_INPUT_PORT_Y,
                                },
                                GRAPH_INPUT_PORT_Y,
                            ) {
                                path {
                                    key: "blueprint-link-{link.id}",
                                    class: match link.kind {
                                        BlueprintLinkKind::Data { .. } => {
                                            "graph-route-path blueprint-data-link".to_owned()
                                        }
                                        BlueprintLinkKind::Flow { signal } => format!(
                                            "graph-route-path signal-{}",
                                            control_signal_value(signal),
                                        ),
                                    },
                                    d: "{path_data}",
                                    marker_end: "url(#blueprint-flow-arrow)",
                                }
                            }
                        }
                    }
                    div { class: "graph-freeform-nodes",
                        for node in &blueprint.nodes {
                            {
                                let can_start_route = !matches!(node.kind, BlueprintNodeKind::Terminal);
                                let point = rendered_positions
                                    .get(&node.id)
                                    .copied()
                                    .or_else(|| layout.canvas_point(&node.id))
                                    .unwrap_or_default();
                                let node_id = node.id.clone();
                                let outgoing_signals = blueprint
                                    .links
                                    .iter()
                                    .filter_map(|link| {
                                        (link.source_node_id == node.id).then_some(&link.kind)
                                    })
                                    .filter_map(|kind| match kind {
                                        BlueprintLinkKind::Flow { signal } => Some(*signal),
                                        BlueprintLinkKind::Data { .. } => None,
                                    })
                                    .collect::<Vec<_>>();
                                let branch_signals = branch_signal_options(
                                    blueprint_signals_for_node(Some(&node.kind)),
                                    primary_blueprint_branch_signals(&node.kind),
                                    &outgoing_signals,
                                    Some(flow_signal),
                                );
                                let node_style =
                                    graph_freeform_node_style(point, branch_signals.len());
                                rsx! {
                                    button {
                                        key: "blueprint-node-{node.id}",
                                        class: match (
                                            node.id == dragged_node_value && pointer_drag_value,
                                            node.id == drop_target_value,
                                            node.id == active_node_id,
                                            layout.is_reachable(&node.id),
                                        ) {
                                            (true, _, _, _) => "graph-node graph-freeform-node blueprint-node dragging",
                                            (_, true, _, _) => "graph-node graph-freeform-node blueprint-node drop-target",
                                            (_, _, true, _) => "graph-node graph-freeform-node blueprint-node active",
                                            (_, _, _, false) => "graph-node graph-freeform-node blueprint-node unreachable",
                                            _ => "graph-node graph-freeform-node blueprint-node",
                                        },
                                        style: "{node_style}",
                                        title: "Drag the card to move it. Use the right socket to connect it.",
                                        onclick: move |_| on_node_select.call(node_id.clone()),
                                        onpointerdown: {
                                            let node_id = node.id.clone();
                                            let start_position = point;
                                            move |event| {
                                                if dragged_route_source_id.read().is_empty() {
                                                    let origin = event.client_coordinates();
                                                    dragged_node_id.set(node_id.clone());
                                                    dragged_route_signal.set(None);
                                                    drag_origin.set(Some(CanvasPoint { x: origin.x, y: origin.y }));
                                                    drag_start_position.set(Some(start_position));
                                                    pointer_drag_active.set(false);
                                                    drag_preview.set(None);
                                                    on_node_select.call(node_id.clone());
                                                }
                                            }
                                        },
                                        onpointerenter: {
                                            let destination = node.id.clone();
                                            move |_| {
                                                let source = dragged_route_source_id.read();
                                                if !source.is_empty() && source.as_str() != destination {
                                                    drop_target_node_id.set(destination.clone());
                                                }
                                            }
                                        },
                                        onpointerup: {
                                            let destination = node.id.clone();
                                            move |event| {
                                                let source = dragged_route_source_id.read().clone();
                                                let signal = *dragged_route_signal.read();
                                                if !source.is_empty() {
                                                    event.stop_propagation();
                                                    if source != destination
                                                        && let Some(signal) = signal
                                                    {
                                                        on_flow_connect.call((source, destination.clone(), signal));
                                                    }
                                                    dragged_route_source_id.set(String::new());
                                                    dragged_route_signal.set(None);
                                                    drop_target_node_id.set(String::new());
                                                }
                                            }
                                        },
                                        span { class: "graph-node-socket graph-node-socket-input", aria_hidden: "true" }
                                        if node.id == blueprint.entry_node_id {
                                            span { class: "graph-node-entry", "Entry" }
                                        } else if !layout.is_reachable(&node.id) {
                                            span { class: "graph-node-entry graph-node-unconnected", "Connect me" }
                                        }
                                        strong { "{node.id}" }
                                        small { "{blueprint_node_kind_label(&node.kind)}" }
                                        div { class: "blueprint-node-sockets",
                                            for socket in &node.inputs {
                                                span { class: "blueprint-socket input", "{note_socket_label(*socket)}" }
                                            }
                                            for socket in &node.outputs {
                                                span { class: "blueprint-socket output", "{note_socket_label(*socket)}" }
                                            }
                                        }
                                        if can_start_route {
                                            div { class: "graph-node-branches", aria_label: "Branches from {node.id}",
                                                for (branch_index, signal) in branch_signals.into_iter().enumerate() {
                                                    {
                                                        let branch_style = graph_branch_pin_style(branch_index);
                                                        let destination = blueprint.links.iter().find_map(|link| {
                                                            (link.source_node_id == node.id
                                                                && matches!(link.kind, BlueprintLinkKind::Flow { signal: route_signal } if route_signal == signal))
                                                                .then_some(link.destination_node_id.clone())
                                                        });
                                                        let branch_class = if destination.is_some() {
                                                            "graph-node-branch-pin connected"
                                                        } else {
                                                            "graph-node-branch-pin"
                                                        };
                                                        let can_connect = destination.is_none();
                                                        let signal_label = control_signal_label(signal);
                                                        let branch_title = destination.as_ref().map_or_else(
                                                            || format!("Drag the {signal_label} branch to another node"),
                                                            |destination| format!("{signal_label} connects to {destination}. Drag to replace after removing it."),
                                                        );
                                                        rsx! {
                                                            span {
                                                                class: "{branch_class}",
                                                                style: "{branch_style}",
                                                                title: "{branch_title}",
                                                                aria_label: "Connect {signal_label} from {node.id}",
                                                                onpointerdown: {
                                                                    let node_id = node.id.clone();
                                                                    move |event| {
                                                                        event.stop_propagation();
                                                                        if !can_connect {
                                                                            return;
                                                                        }
                                                                        dragged_node_id.set(String::new());
                                                                        drag_origin.set(None);
                                                                        drag_start_position.set(None);
                                                                        pointer_drag_active.set(false);
                                                                        drag_preview.set(None);
                                                                        dragged_route_source_id.set(node_id.clone());
                                                                        dragged_route_signal.set(Some(signal));
                                                                    }
                                                                },
                                                                span { class: "graph-node-branch-label", "{signal_label}" }
                                                                span { class: "graph-node-socket graph-node-socket-output", aria_hidden: "true" }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if let Some(pending) = branch_spawn_value {
                        div {
                            class: "graph-spawn-menu graph-branch-spawn-menu",
                            style: "{branch_spawn_style}",
                            strong { "Create branch destination" }
                            small { "{control_signal_label(pending.signal)} from {pending.source_node_id}" }
                            for (kind_value, label) in [
                                ("gate", "Gate"),
                                ("audit", "Audit"),
                                ("approval", "Approval"),
                                ("terminal", "Terminal"),
                            ] {
                                {
                                    let pending_for_spawn = pending.clone();
                                    let blueprint_for_spawn = blueprint.clone();
                                    let layout_id_for_spawn = spawn_layout_id.clone();
                                    rsx! {
                                        button {
                                            onclick: move |event| {
                                                event.stop_propagation();
                                                if let Some(node) = blueprint_stage_node(&blueprint_for_spawn, kind_value) {
                                                    let node_id = node.id.clone();
                                                    node_positions.write().insert(node_id, pending_for_spawn.point);
                                                    on_layout_save.call(graph_canvas_layout_from_positions(
                                                        &layout_id_for_spawn,
                                                        &node_positions.read(),
                                                    ));
                                                    on_branch_spawn.call((
                                                        pending_for_spawn.source_node_id.clone(),
                                                        pending_for_spawn.signal,
                                                        node,
                                                    ));
                                                }
                                                branch_spawn_menu.set(None);
                                            },
                                            "{label}"
                                        }
                                    }
                                }
                            }
                            button {
                                class: "graph-spawn-menu-cancel",
                                onclick: move |event| {
                                    event.stop_propagation();
                                    branch_spawn_menu.set(None);
                                },
                                "Cancel"
                            }
                        }
                    }
                }
            }
            div { class: "graph-route-map blueprint-link-map",
                for link in &blueprint.links {
                    div { class: "graph-route-edge", key: "blueprint-edge-{link.id}",
                        button {
                            class: "graph-route-node",
                            onclick: {
                                let node_id = link.source_node_id.clone();
                                move |_| on_node_select.call(node_id.clone())
                            },
                            "{link.source_node_id}"
                        }
                        span { class: "graph-route-line",
                            small { "{blueprint_link_label(link.kind)}" }
                            span { "→" }
                        }
                        button {
                            class: "graph-route-node",
                            onclick: {
                                let node_id = link.destination_node_id.clone();
                                move |_| on_node_select.call(node_id.clone())
                            },
                            "{link.destination_node_id}"
                        }
                        button {
                            class: "graph-route-remove",
                            aria_label: "Remove Blueprint connection {link.id}",
                            onclick: {
                                let link_id = link.id.clone();
                                move |_| on_remove_link.call(link_id.clone())
                            },
                            "×"
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn BlueprintInspector(
    blueprint: OrchestrationBlueprintRevision,
    node_id: String,
    approach_notes: Vec<ApproachNoteManifest>,
) -> Element {
    let node = blueprint.nodes.iter().find(|node| node.id == node_id);
    let note = node.and_then(|node| match &node.kind {
        BlueprintNodeKind::Approach { approach_id } => approach_notes
            .iter()
            .find(|note| note.approach_id == *approach_id),
        _ => None,
    });
    rsx! {
        aside { class: "blueprint-inspector",
            div { class: "blueprint-pane-title", span { "Inspector" } }
            if let Some(node) = node {
                p { class: "project-id", "{node.id}" }
                h4 { "{blueprint_node_kind_label(&node.kind)}" }
                dl {
                    div { dt { "Inputs" } dd { "{socket_list_label(&node.inputs)}" } }
                    div { dt { "Outputs" } dd { "{socket_list_label(&node.outputs)}" } }
                }
                if let BlueprintNodeKind::Approach { approach_id } = &node.kind {
                    if let Some(note) = note {
                        div { class: "blueprint-note-pin",
                            span { "Pinned on application" }
                            code { "{short_fingerprint(&note.fingerprint)}" }
                            small { "{note.absolute_path}" }
                        }
                    } else {
                        p { class: "blueprint-inspector-warning", "Approach Note {approach_id} is not loaded. Applications fail closed." }
                    }
                }
            } else {
                p { class: "blueprint-pane-empty", "Select a node to inspect its portable contract." }
            }
        }
    }
}

#[component]
fn BlueprintApplicationPreviewPanel(
    blueprint_id: String,
    revision_id: String,
    preview: BlueprintApplicationPreview,
    applications: Vec<BlueprintApplicationReceipt>,
    on_accept: EventHandler<BlueprintApplicationProposal>,
) -> Element {
    let candidates = preview
        .candidates
        .iter()
        .filter(|candidate| {
            candidate.blueprint_id == blueprint_id && candidate.revision_id == revision_id
        })
        .collect::<Vec<_>>();
    let blocked = preview
        .blocked
        .iter()
        .filter(|blocked| {
            blocked.blueprint_id == blueprint_id && blocked.revision_id == revision_id
        })
        .collect::<Vec<_>>();
    rsx! {
        section { class: "blueprint-application-preview", aria_label: "Blueprint application preview",
            header {
                div {
                    span { "Autopilot applicability" }
                    strong { "{candidates.len()} project candidate(s)" }
                }
                small { "Preview only · no binding or Runner started" }
            }
            div { class: "blueprint-application-grid",
                for candidate in candidates {
                    {
                        let applied = applications.iter().any(|application| {
                            application.application.blueprint_id == candidate.blueprint_id
                                && application.application.revision_id == candidate.revision_id
                                && application.application.project_id == candidate.project_id
                                && application.application.work_item_id == candidate.work_item.id
                        });
                        let proposal = candidate.clone();
                        rsx! {
                            article { class: "blueprint-application-card candidate", key: "candidate-{candidate.project_id}",
                                span { if applied { "Applied" } else { "Applicable" } }
                                h4 { "{candidate.project_name}" }
                                strong { "{candidate.work_item.id} · {candidate.work_item.title}" }
                                small { "Agent candidate · {candidate.agent_role} ({candidate.agent_profile_id})" }
                                small { "{candidate.approach_notes.len()} Note pin(s) · {approach_risk_label(candidate.highest_risk)} risk" }
                                button {
                                    disabled: applied,
                                    onclick: move |_| on_accept.call(proposal.clone()),
                                    if applied { "Runtime binding pinned" } else { "Apply without starting" }
                                }
                            }
                        }
                    }
                }
                for item in blocked {
                    {
                        let project_label = item.project_id.as_deref().unwrap_or("Blueprint");
                        rsx! {
                            article { class: "blueprint-application-card blocked", key: "blocked-{item.project_id:?}",
                                span { "Not currently applicable" }
                                h4 { "{project_label}" }
                                p { "{blueprint_blocked_message(&item.reason)}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn add_note_to_blueprint_draft(
    draft: &mut Signal<Option<BlueprintDraft>>,
    note: &ApproachNoteManifest,
) -> Result<String, String> {
    draft.write().as_mut().map_or_else(
        || Err("Blueprint draft is unavailable".to_owned()),
        |draft| {
            let node_id = unique_blueprint_node_id(draft.blueprint(), &note.approach_id);
            draft
                .add_approach(node_id.clone(), note)
                .map_err(|error| error.to_string())?;
            Ok(node_id)
        },
    )
}

fn connect_blueprint_draft(
    draft: &mut Signal<Option<BlueprintDraft>>,
    source_node_id: String,
    destination_node_id: String,
    kind: BlueprintLinkKind,
) -> Result<(), String> {
    if source_node_id == destination_node_id {
        return Err("Choose two different Blueprint nodes.".to_owned());
    }
    let link_id = next_blueprint_link_id();
    draft.write().as_mut().map_or_else(
        || Err("Blueprint draft is unavailable".to_owned()),
        |draft| {
            draft
                .connect(link_id, source_node_id, destination_node_id, kind)
                .map_err(|error| error.to_string())
        },
    )
}

fn unique_blueprint_node_id(blueprint: &OrchestrationBlueprintRevision, base: &str) -> String {
    if !blueprint.nodes.iter().any(|node| node.id == base) {
        return base.to_owned();
    }
    for number in 2..=999_u16 {
        let suffix = format!("-{number}");
        let prefix_length = 64_usize.saturating_sub(suffix.len());
        let prefix = base.chars().take(prefix_length).collect::<String>();
        let candidate = format!("{prefix}{suffix}");
        if !blueprint.nodes.iter().any(|node| node.id == candidate) {
            return candidate;
        }
    }
    format!("node-{}", next_blueprint_nonce())
}

fn blueprint_stage_node(
    blueprint: &OrchestrationBlueprintRevision,
    stage_kind: &str,
) -> Option<BlueprintNode> {
    let (base, kind, inputs, outputs) = match stage_kind {
        "gate" => (
            "gate",
            BlueprintNodeKind::Gate,
            vec![NoteSocketKind::Evidence],
            vec![NoteSocketKind::Signal],
        ),
        "audit" => (
            "audit",
            BlueprintNodeKind::Audit,
            vec![NoteSocketKind::Evidence],
            vec![NoteSocketKind::Evidence],
        ),
        "approval" => (
            "approval",
            BlueprintNodeKind::Approval,
            vec![NoteSocketKind::Evidence],
            vec![NoteSocketKind::Approval],
        ),
        "terminal" => (
            "terminal",
            BlueprintNodeKind::Terminal,
            vec![
                NoteSocketKind::Evidence,
                NoteSocketKind::Artifact,
                NoteSocketKind::Signal,
                NoteSocketKind::Approval,
            ],
            Vec::new(),
        ),
        _ => return None,
    };
    Some(BlueprintNode {
        id: unique_blueprint_node_id(blueprint, base),
        kind,
        inputs,
        outputs,
    })
}

fn next_blueprint_link_id() -> String {
    format!("link-{}", next_blueprint_nonce())
}

fn next_blueprint_nonce() -> String {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}-{}", std::process::id(), elapsed.as_millis())
}

fn blueprint_signals_for_node(kind: Option<&BlueprintNodeKind>) -> &'static [ControlSignal] {
    match kind {
        Some(BlueprintNodeKind::Approval) => &APPROVAL_GRAPH_SIGNALS,
        Some(BlueprintNodeKind::Terminal) | None => &[],
        Some(
            BlueprintNodeKind::Approach { .. }
            | BlueprintNodeKind::Gate
            | BlueprintNodeKind::Audit
            | BlueprintNodeKind::ProjectSelector { .. }
            | BlueprintNodeKind::PostAction { .. },
        ) => &STANDARD_GRAPH_SIGNALS,
    }
}

fn note_socket_value(socket: NoteSocketKind) -> &'static str {
    match socket {
        NoteSocketKind::Context => "context",
        NoteSocketKind::WorkItem => "work_item",
        NoteSocketKind::Evidence => "evidence",
        NoteSocketKind::Artifact => "artifact",
        NoteSocketKind::Signal => "signal",
        NoteSocketKind::Approval => "approval",
    }
}

fn parse_note_socket(value: &str) -> Option<NoteSocketKind> {
    [
        NoteSocketKind::Context,
        NoteSocketKind::WorkItem,
        NoteSocketKind::Evidence,
        NoteSocketKind::Artifact,
        NoteSocketKind::Signal,
        NoteSocketKind::Approval,
    ]
    .into_iter()
    .find(|socket| note_socket_value(*socket) == value)
}

fn blueprint_link_label(kind: BlueprintLinkKind) -> String {
    match kind {
        BlueprintLinkKind::Flow { signal } => control_signal_label(signal).to_owned(),
        BlueprintLinkKind::Data { socket } => format!("{} data", note_socket_label(socket)),
    }
}

fn blueprint_node_kind_label(kind: &BlueprintNodeKind) -> String {
    match kind {
        BlueprintNodeKind::Approach { approach_id } => format!("Approach · {approach_id}"),
        BlueprintNodeKind::Gate => "Gate".to_owned(),
        BlueprintNodeKind::Audit => "Audit".to_owned(),
        BlueprintNodeKind::Approval => "Approval".to_owned(),
        BlueprintNodeKind::ProjectSelector {
            target_blueprint_id,
            target_revision_id,
            ..
        } => format!("Select projects · {target_blueprint_id}/{target_revision_id}"),
        BlueprintNodeKind::PostAction { .. } => "Post action · Record summary".to_owned(),
        BlueprintNodeKind::Terminal => "Terminal".to_owned(),
    }
}

fn note_socket_label(socket: NoteSocketKind) -> &'static str {
    match socket {
        NoteSocketKind::Context => "Context",
        NoteSocketKind::WorkItem => "Work item",
        NoteSocketKind::Evidence => "Evidence",
        NoteSocketKind::Artifact => "Artifact",
        NoteSocketKind::Signal => "Signal",
        NoteSocketKind::Approval => "Approval",
    }
}

fn socket_list_label(sockets: &[NoteSocketKind]) -> String {
    if sockets.is_empty() {
        "None".to_owned()
    } else {
        sockets
            .iter()
            .map(|socket| note_socket_label(*socket))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn approach_risk_label(risk: ApproachRisk) -> &'static str {
    match risk {
        ApproachRisk::Low => "Low",
        ApproachRisk::Moderate => "Moderate",
        ApproachRisk::High => "High",
        ApproachRisk::Critical => "Critical",
    }
}

fn short_fingerprint(fingerprint: &str) -> &str {
    fingerprint.get(..12).unwrap_or(fingerprint)
}

fn blueprint_blocked_message(reason: &BlueprintApplicationBlockedReason) -> String {
    match reason {
        BlueprintApplicationBlockedReason::InvalidBlueprint(_) => {
            "The Blueprint revision is invalid.".to_owned()
        }
        BlueprintApplicationBlockedReason::BlueprintScopeNotProject => {
            "This Blueprint coordinates a portfolio rather than one project.".to_owned()
        }
        BlueprintApplicationBlockedReason::DuplicateApproachNote { approach_id } => {
            format!("More than one Note declares {approach_id}.")
        }
        BlueprintApplicationBlockedReason::ApproachNoteNotFound { approach_id } => {
            format!("Load the Markdown Approach Note {approach_id}.")
        }
        BlueprintApplicationBlockedReason::InvalidApproachNote { approach_id, .. } => {
            format!("Approach Note {approach_id} is invalid.")
        }
        BlueprintApplicationBlockedReason::SocketContractMismatch { approach_id, .. } => {
            format!("Approach Note {approach_id} no longer matches this node's sockets.")
        }
        BlueprintApplicationBlockedReason::ExecutionWorkspaceNotFound => {
            "Connect an Execution workspace for this project.".to_owned()
        }
        BlueprintApplicationBlockedReason::NoCandidate(reason) => no_candidate_message(*reason),
        BlueprintApplicationBlockedReason::AutopilotStopped(reason) => {
            autopilot_stop_message(reason)
        }
        BlueprintApplicationBlockedReason::SelectedWorkItemNotFound { work_item_id } => {
            format!("Selected Work item {work_item_id} is no longer available.")
        }
    }
}

#[component]
fn PortfolioOrchestrationPanel(
    revisions: Vec<PortfolioOrchestrationRevision>,
    graph_canvas_layouts: Vec<GraphCanvasLayout>,
    preview: Option<PortfolioStepPreview>,
    runs: Vec<PortfolioRun>,
    steps: Vec<PortfolioRunStep>,
    schedule_controls: Vec<PortfolioScheduleControl>,
    projects: Vec<ProjectSummary>,
    notice: Option<String>,
    on_preview: EventHandler<(String, String)>,
    on_tick: EventHandler<(String, String)>,
    on_schedule_control: EventHandler<(String, String, bool)>,
    on_approval: EventHandler<(String, String, PortfolioApprovalDecision)>,
    on_save_revision: EventHandler<PortfolioOrchestrationRevision>,
    on_graph_canvas_layout: EventHandler<GraphCanvasLayout>,
) -> Element {
    let initial_revision = revisions.last().map_or_else(String::new, |revision| {
        format!("{}|{}", revision.orchestration_id, revision.revision_id)
    });
    let mut selected_revision = use_signal(move || initial_revision);
    let mut editor_generation = use_signal(|| 0_u64);
    let selected_value = selected_revision.read().clone();
    let selected = revisions
        .iter()
        .find(|revision| {
            format!("{}|{}", revision.orchestration_id, revision.revision_id) == selected_value
        })
        .or_else(|| revisions.last());
    rsx! {
        section { class: "section-heading portfolio-orchestration-heading",
            div {
                p { class: "kicker", "Portfolio autopilot" }
                h3 { "Orchestration graph" }
            }
            span { "One safe step per tick" }
        }
        section { class: "portfolio-orchestration-panel", aria_label: "Portfolio orchestration graph",
            if let Some(revision) = selected {
                {
                    let run = runs.iter().find(|run| {
                        run.orchestration_id == revision.orchestration_id
                            && run.revision_id == revision.revision_id
                    });
                    let run_steps = run.map_or_else(Vec::new, |run| {
                        steps.iter().filter(|step| step.run_id == run.run_id).collect::<Vec<_>>()
                    });
                    let automatic_ticks_enabled = schedule_controls
                        .iter()
                        .find(|control| {
                            control.orchestration_id == revision.orchestration_id
                                && control.revision_id == revision.revision_id
                        })
                        .is_some_and(|control| control.automatic_ticks_enabled);
                    let manual_tick_disabled = run.is_some_and(|run| {
                        matches!(
                            run.status,
                            PortfolioRunStatus::WaitingApproval | PortfolioRunStatus::Paused
                        )
                    });
                    rsx! {
                header { class: "portfolio-orchestration-header",
                    div {
                        p { class: "project-id", "{revision.orchestration_id} · {revision.revision_id}" }
                        h4 { "{revision.name}" }
                    }
                    div { class: "portfolio-orchestration-schedule",
                        strong { "{portfolio_schedule_label(revision.schedule)}" }
                        small {
                            if automatic_ticks_enabled {
                                "Automatic ticks are active while Gareji Board is open."
                            } else {
                                "Automatic ticks are paused. The current Run position is preserved."
                            }
                        }
                    }
                }
                if revisions.len() > 1 {
                    label { class: "portfolio-orchestration-select",
                        span { "Orchestration revision" }
                        select {
                            aria_label: "Portfolio orchestration revision",
                            value: "{selected_value}",
                            onchange: move |event| {
                                selected_revision.set(event.value());
                                editor_generation += 1;
                            },
                            for option in &revisions {
                                option {
                                    value: "{option.orchestration_id}|{option.revision_id}",
                                    "{option.name} · {option.revision_id}"
                                }
                            }
                        }
                    }
                }
                PortfolioOrchestrationEditor {
                    source: revision.clone(),
                    graph_canvas_layouts: graph_canvas_layouts.clone(),
                    projects: projects.clone(),
                    generation: editor_generation,
                    on_save_revision,
                    on_graph_canvas_layout,
                }
                if let Some(run) = run {
                    article { class: "portfolio-step-preview",
                        div { class: "portfolio-step-preview-heading",
                            div {
                                span { "Current Portfolio Run" }
                                strong { "{run.current_node_id}" }
                            }
                            code { "{run.run_id}" }
                        }
                        p { "Status · {portfolio_run_status_label(run.status)} · {run.completed_steps} recorded step(s)" }
                        if let Some(due) = run.next_tick_at_epoch_seconds {
                            small { "Next scheduler checkpoint · Unix {due}" }
                        }
                        if !run_steps.is_empty() {
                            div { class: "portfolio-orchestration-routes",
                                for step in run_steps.iter().rev().take(4) {
                                    {
                                        let signal_label = step.signal.map_or("Stopped", portfolio_signal_label);
                                        let destination = step.destination_node_id.as_deref().unwrap_or("-");
                                        rsx! { span { key: "portfolio-run-step-{step.sequence}",
                                            code { "#{step.sequence} {step.node_id}" }
                                            small { "{signal_label}" }
                                            code { "{destination}" }
                                        } }
                                    }
                                }
                            }
                        }
                    }
                    if run.status == PortfolioRunStatus::WaitingApproval {
                        div { class: "portfolio-orchestration-actions",
                            div {
                                strong { "Human decision required" }
                                small { "Only the declared Approved or Rejected route can advance this Run." }
                            }
                            button {
                                onclick: {
                                    let request = (
                                        revision.orchestration_id.clone(),
                                        revision.revision_id.clone(),
                                        PortfolioApprovalDecision::Approved,
                                    );
                                    move |_| on_approval.call(request.clone())
                                },
                                "Approve and continue"
                            }
                            button {
                                class: "secondary-button",
                                onclick: {
                                    let request = (
                                        revision.orchestration_id.clone(),
                                        revision.revision_id.clone(),
                                        PortfolioApprovalDecision::Rejected,
                                    );
                                    move |_| on_approval.call(request.clone())
                                },
                                "Reject"
                            }
                        }
                    }
                }
                div { class: "portfolio-orchestration-actions",
                    div {
                        strong { "Advance one bounded node" }
                        small { "Project Selector resolves across managed projects. A tick records and routes only; it never starts a Runner." }
                    }
                    button {
                        disabled: manual_tick_disabled,
                        onclick: {
                            let request = (
                                revision.orchestration_id.clone(),
                                revision.revision_id.clone(),
                            );
                            move |_| on_preview.call(request.clone())
                        },
                        "Preview next portfolio step"
                    }
                    button {
                        onclick: {
                            let request = (
                                revision.orchestration_id.clone(),
                                revision.revision_id.clone(),
                            );
                            move |_| on_tick.call(request.clone())
                        },
                        "Run one node now"
                    }
                    if revision.schedule.interval_seconds().is_some() {
                        button {
                            class: "secondary-button",
                            onclick: {
                                let request = (
                                    revision.orchestration_id.clone(),
                                    revision.revision_id.clone(),
                                    !automatic_ticks_enabled,
                                );
                                move |_| on_schedule_control.call(request.clone())
                            },
                            if automatic_ticks_enabled {
                                "Pause automatic ticks"
                            } else {
                                "Resume automatic ticks"
                            }
                        }
                    }
                }
                if let Some(notice) = &notice {
                    p { class: "portfolio-step-note", "{notice}" }
                }
                if let Some(preview) = preview.as_ref().filter(|preview| {
                    preview.orchestration_id == revision.orchestration_id
                        && preview.revision_id == revision.revision_id
                }) {
                    PortfolioStepPreviewCard { preview: preview.clone() }
                }
                    }
                }
            } else {
                p { class: "portfolio-graph-empty", "No Portfolio orchestration revision is available." }
            }
        }
    }
}

#[component]
fn PortfolioOrchestrationEditor(
    source: PortfolioOrchestrationRevision,
    graph_canvas_layouts: Vec<GraphCanvasLayout>,
    projects: Vec<ProjectSummary>,
    generation: Signal<u64>,
    on_save_revision: EventHandler<PortfolioOrchestrationRevision>,
    on_graph_canvas_layout: EventHandler<GraphCanvasLayout>,
) -> Element {
    let canvas_layout_id = workspace_canvas_layout_id("portfolio", &source.orchestration_id);
    let canvas_layout = graph_canvas_layouts
        .iter()
        .find(|layout| layout.graph_id == canvas_layout_id)
        .cloned();
    let draft_source = source.clone();
    let mut draft = use_signal(move || PortfolioDraft::new(draft_source).ok());
    let entry_node_id = source.entry_node_id.clone();
    let mut selected_node_id = use_signal(move || entry_node_id);
    let mut new_node_id = use_signal(String::new);
    let mut selector_all_managed = use_signal(|| true);
    let mut included_project_ids = use_signal(Vec::<String>::new);
    let mut connection_target_id = use_signal(String::new);
    let mut connection_signal = use_signal(|| "completed".to_owned());
    let schedule_kind_value = match source.schedule {
        PortfolioSchedule::Manual => "manual",
        PortfolioSchedule::Interval { .. } => "interval",
    };
    let schedule_minutes_value = match source.schedule {
        PortfolioSchedule::Manual => "60".to_owned(),
        PortfolioSchedule::Interval { every_minutes, .. } => every_minutes.to_string(),
    };
    let schedule_enabled_value = matches!(
        source.schedule,
        PortfolioSchedule::Interval { enabled: true, .. }
    );
    let mut schedule_kind = use_signal(move || schedule_kind_value.to_owned());
    let mut schedule_minutes = use_signal(move || schedule_minutes_value);
    let mut schedule_enabled = use_signal(move || schedule_enabled_value);
    let candidate_value = next_portfolio_revision_suggestion(&source.revision_id);
    let mut candidate_revision_id = use_signal(move || candidate_value);
    let mut editor_notice = use_signal(String::new);
    let reset_source = source.clone();
    use_effect(move || {
        let _generation = generation();
        draft.set(PortfolioDraft::new(reset_source.clone()).ok());
        selected_node_id.set(reset_source.entry_node_id.clone());
        new_node_id.set(String::new());
        connection_target_id.set(String::new());
        connection_signal.set("completed".to_owned());
        let (kind, minutes, enabled) = match reset_source.schedule {
            PortfolioSchedule::Manual => ("manual".to_owned(), "60".to_owned(), false),
            PortfolioSchedule::Interval {
                every_minutes,
                enabled,
            } => ("interval".to_owned(), every_minutes.to_string(), enabled),
        };
        schedule_kind.set(kind);
        schedule_minutes.set(minutes);
        schedule_enabled.set(enabled);
        candidate_revision_id.set(next_portfolio_revision_suggestion(
            &reset_source.revision_id,
        ));
        editor_notice.set(String::new());
    });

    let orchestration = draft
        .read()
        .as_ref()
        .map_or_else(|| source.clone(), |draft| draft.orchestration().clone());
    let selected_node_value = selected_node_id.read().clone();
    let selected_node = orchestration
        .nodes
        .iter()
        .find(|node| node.id == selected_node_value);
    let allowed_signals = selected_node
        .map(|node| portfolio_allowed_signals(&node.kind))
        .unwrap_or_default();
    let requested_signal = connection_signal.read().clone();
    let effective_signal = allowed_signals
        .iter()
        .copied()
        .find(|signal| portfolio_signal_value(*signal) == requested_signal)
        .or_else(|| allowed_signals.first().copied());
    let effective_signal_value = effective_signal.map_or("", portfolio_signal_value);
    let connection_target_value = connection_target_id.read().clone();
    let has_changes = draft
        .read()
        .as_ref()
        .is_some_and(PortfolioDraft::has_changes);
    let can_undo = draft.read().as_ref().is_some_and(PortfolioDraft::can_undo);
    let candidate_id = candidate_revision_id.read().clone();
    let can_save = draft
        .read()
        .as_ref()
        .is_some_and(|draft| draft.revision(candidate_id.trim()).is_ok());

    rsx! {
        section { class: "portfolio-editor", aria_label: "Portfolio graph editor",
            header { class: "graph-canvas-heading",
                div {
                    strong { "Edit this Portfolio revision" }
                    small { "Draft changes stay local until an immutable candidate revision is saved." }
                }
                div { class: "graph-canvas-actions",
                    button {
                        disabled: !can_undo,
                        onclick: move |_| {
                            let result = apply_portfolio_draft(&mut draft, PortfolioDraft::undo);
                            set_portfolio_editor_notice(&mut editor_notice, result, "Undid the last graph edit.");
                        },
                        "Undo"
                    }
                    button {
                        disabled: !has_changes,
                        onclick: move |_| {
                            let result = apply_portfolio_draft(&mut draft, PortfolioDraft::reset);
                            if result.is_ok() {
                                selected_node_id.set(source.entry_node_id.clone());
                            }
                            set_portfolio_editor_notice(&mut editor_notice, result, "Reset to the stored revision.");
                        },
                        "Reset"
                    }
                }
            }

            div { class: "portfolio-editor-grid",
                aside { class: "portfolio-editor-tools",
                    h4 { "Schedule" }
                    label {
                        span { "Cadence" }
                        select {
                            value: "{schedule_kind}",
                            onchange: move |event| schedule_kind.set(event.value()),
                            option { value: "manual", "Manual only" }
                            option { value: "interval", "Interval" }
                        }
                    }
                    if schedule_kind.read().as_str() == "interval" {
                        label {
                            span { "Minutes" }
                            input {
                                r#type: "number",
                                min: "5",
                                max: "10080",
                                value: "{schedule_minutes}",
                                oninput: move |event| schedule_minutes.set(event.value()),
                            }
                        }
                        label { class: "portfolio-editor-check",
                            input {
                                r#type: "checkbox",
                                checked: schedule_enabled(),
                                onchange: move |event| schedule_enabled.set(event.checked()),
                            }
                            span { "Start automatic ticks" }
                        }
                    }
                    button {
                        onclick: move |_| {
                            let result = portfolio_schedule_from_editor(
                                &schedule_kind.read(),
                                &schedule_minutes.read(),
                                schedule_enabled(),
                            )
                            .and_then(|schedule| {
                                apply_portfolio_draft(&mut draft, |draft| draft.set_schedule(schedule))
                            });
                            set_portfolio_editor_notice(&mut editor_notice, result, "Updated the draft schedule.");
                        },
                        "Apply schedule"
                    }

                    h4 { "Add a node" }
                    label {
                        span { "Node name" }
                        input {
                            value: "{new_node_id}",
                            placeholder: "review-projects",
                            oninput: move |event| new_node_id.set(event.value()),
                        }
                    }
                    label { class: "portfolio-editor-check",
                        input {
                            r#type: "checkbox",
                            checked: selector_all_managed(),
                            onchange: move |event| selector_all_managed.set(event.checked()),
                        }
                        span { "Selector uses all managed projects" }
                    }
                    if !selector_all_managed() {
                        div { class: "portfolio-editor-projects",
                            for project in &projects {
                                {
                                    let project_id = project.id.clone();
                                    let checked = included_project_ids.read().contains(&project.id);
                                    rsx! { label { key: "selector-project-{project.id}",
                                        input {
                                            r#type: "checkbox",
                                            checked,
                                            onchange: move |event| {
                                                let mut selected = included_project_ids.write();
                                                if event.checked() {
                                                    if !selected.contains(&project_id) {
                                                        selected.push(project_id.clone());
                                                    }
                                                } else {
                                                    selected.retain(|id| id != &project_id);
                                                }
                                            },
                                        }
                                        span { "{project.name}" }
                                    } }
                                }
                            }
                        }
                    }
                    div { class: "portfolio-editor-add-actions",
                        button {
                            onclick: move |_| {
                                let node_id = new_node_id.read().trim().to_owned();
                                let selector = if selector_all_managed() {
                                    PortfolioProjectSelector::AllManaged
                                } else {
                                    PortfolioProjectSelector::Include {
                                        project_ids: included_project_ids.read().clone(),
                                    }
                                };
                                let result = apply_portfolio_draft(&mut draft, |draft| {
                                    draft.add_project_selector(node_id.clone(), selector)
                                });
                                if result.is_ok() {
                                    selected_node_id.set(node_id);
                                    new_node_id.set(String::new());
                                }
                                set_portfolio_editor_notice(&mut editor_notice, result, "Added a Project Selector.");
                            },
                            "Project Selector"
                        }
                        button {
                            onclick: move |_| {
                                let node_id = new_node_id.read().trim().to_owned();
                                let result = apply_portfolio_draft(&mut draft, |draft| {
                                    draft.add_post_action(node_id.clone(), PortfolioPostAction::RecordSummary)
                                });
                                if result.is_ok() {
                                    selected_node_id.set(node_id);
                                    new_node_id.set(String::new());
                                }
                                set_portfolio_editor_notice(&mut editor_notice, result, "Added a Summary action.");
                            },
                            "Summary"
                        }
                        button {
                            onclick: move |_| {
                                let node_id = new_node_id.read().trim().to_owned();
                                let result = apply_portfolio_draft(&mut draft, |draft| {
                                    draft.add_post_action(node_id.clone(), PortfolioPostAction::RequestApproval)
                                });
                                if result.is_ok() {
                                    selected_node_id.set(node_id);
                                    new_node_id.set(String::new());
                                }
                                set_portfolio_editor_notice(&mut editor_notice, result, "Added an Approval action.");
                            },
                            "Approval"
                        }
                        button {
                            onclick: move |_| {
                                let node_id = new_node_id.read().trim().to_owned();
                                let result = apply_portfolio_draft(&mut draft, |draft| {
                                    draft.add_terminal(node_id.clone())
                                });
                                if result.is_ok() {
                                    selected_node_id.set(node_id);
                                    new_node_id.set(String::new());
                                }
                                set_portfolio_editor_notice(&mut editor_notice, result, "Added a Terminal.");
                            },
                            "Terminal"
                        }
                    }
                }

                div { class: "portfolio-editor-canvas",
                    PortfolioOrchestrationCanvas {
                        key: "{canvas_layout_id}",
                        revision: orchestration.clone(),
                        canvas_layout: canvas_layout.clone(),
                        canvas_layout_id: canvas_layout_id.clone(),
                        selected_node_id: selected_node_value.clone(),
                        connect_signal: effective_signal,
                        on_node_select: move |node_id| selected_node_id.set(node_id),
                        on_flow_connect: move |(source, destination, signal): (String, String, PortfolioSignal)| {
                            let route_id = next_portfolio_route_id(draft.read().as_ref(), &source, &destination);
                            let result = apply_portfolio_draft(&mut draft, |draft| {
                                draft.connect(route_id, source, destination, signal)
                            });
                            set_portfolio_editor_notice(&mut editor_notice, result, "Connected the two Portfolio nodes.");
                        },
                        on_layout_save: on_graph_canvas_layout,
                    }
                    if !editor_notice.read().is_empty() {
                        p { class: "graph-editor-notice", "{editor_notice}" }
                    }
                }

                aside { class: "portfolio-editor-tools",
                    h4 { "Selected node" }
                    if let Some(node) = selected_node {
                        strong { "{node.id}" }
                        small { "{portfolio_node_kind_label(&node.kind)}" }
                        if node.id != orchestration.entry_node_id {
                            button {
                                class: "portfolio-editor-remove",
                                onclick: {
                                    let node_id = node.id.clone();
                                    move |_| {
                                        let result = apply_portfolio_draft(&mut draft, |draft| {
                                            draft.remove_node(&node_id)
                                        });
                                        if result.is_ok() {
                                            selected_node_id.set(orchestration.entry_node_id.clone());
                                        }
                                        set_portfolio_editor_notice(&mut editor_notice, result, "Removed the node and its routes.");
                                    }
                                },
                                "Remove node"
                            }
                        }
                    }

                    h4 { "Connect selected node" }
                    label {
                        span { "Outcome" }
                        select {
                            disabled: allowed_signals.is_empty(),
                            value: "{effective_signal_value}",
                            onchange: move |event| connection_signal.set(event.value()),
                            for signal in &allowed_signals {
                                option {
                                    value: "{portfolio_signal_value(*signal)}",
                                    "{portfolio_signal_label(*signal)}"
                                }
                            }
                        }
                    }
                    label {
                        span { "Next node" }
                        select {
                            value: "{connection_target_value}",
                            onchange: move |event| connection_target_id.set(event.value()),
                            option { value: "", "Choose a node" }
                            for node in &orchestration.nodes {
                                if node.id != selected_node_value {
                                    option { value: "{node.id}", "{node.id}" }
                                }
                            }
                        }
                    }
                    button {
                        disabled: effective_signal.is_none() || connection_target_value.is_empty(),
                        onclick: move |_| {
                            let Some(signal) = effective_signal else { return; };
                            let source = selected_node_id.read().clone();
                            let destination = connection_target_id.read().clone();
                            let route_id = next_portfolio_route_id(draft.read().as_ref(), &source, &destination);
                            let result = apply_portfolio_draft(&mut draft, |draft| {
                                draft.connect(route_id, source, destination, signal)
                            });
                            set_portfolio_editor_notice(&mut editor_notice, result, "Connected the selected nodes.");
                        },
                        "Connect"
                    }

                    if !orchestration.routes.is_empty() {
                        h4 { "Routes" }
                        div { class: "portfolio-editor-routes",
                            for route in &orchestration.routes {
                                div { key: "edit-route-{route.id}",
                                    span { "{route.source_node_id} → {route.destination_node_id}" }
                                    small { "{portfolio_signal_label(route.signal)}" }
                                    button {
                                        onclick: {
                                            let route_id = route.id.clone();
                                            move |_| {
                                                let result = apply_portfolio_draft(&mut draft, |draft| {
                                                    draft.remove_route(&route_id)
                                                });
                                                set_portfolio_editor_notice(&mut editor_notice, result, "Removed the route.");
                                            }
                                        },
                                        "Remove"
                                    }
                                }
                            }
                        }
                    }

                    h4 { "Save candidate" }
                    label {
                        span { "Revision name" }
                        input {
                            value: "{candidate_revision_id}",
                            oninput: move |event| candidate_revision_id.set(event.value()),
                        }
                    }
                    button {
                        disabled: !can_save,
                        onclick: move |_| {
                            let candidate = draft
                                .read()
                                .as_ref()
                                .ok_or_else(|| "Portfolio draft is unavailable.".to_owned())
                                .and_then(|draft| {
                                    draft
                                        .revision(candidate_revision_id.read().trim())
                                        .map_err(|error| error.to_string())
                                });
                            match candidate {
                                Ok(candidate) => on_save_revision.call(candidate),
                                Err(error) => editor_notice.set(error),
                            }
                        },
                        "Save immutable revision"
                    }
                }
            }
        }
    }
}

fn apply_portfolio_draft(
    draft: &mut Signal<Option<PortfolioDraft>>,
    edit: impl FnOnce(&mut PortfolioDraft) -> Result<(), gareji_board_core::PortfolioDraftError>,
) -> Result<(), String> {
    let mut draft = draft.write();
    let draft = draft
        .as_mut()
        .ok_or_else(|| "Portfolio draft is unavailable.".to_owned())?;
    edit(draft).map_err(|error| error.to_string())
}

fn set_portfolio_editor_notice(
    notice: &mut Signal<String>,
    result: Result<(), String>,
    success: &str,
) {
    notice.set(result.map_or_else(|error| error, |()| success.to_owned()));
}

fn portfolio_schedule_from_editor(
    kind: &str,
    minutes: &str,
    enabled: bool,
) -> Result<PortfolioSchedule, String> {
    if kind == "manual" {
        return Ok(PortfolioSchedule::Manual);
    }
    let every_minutes = minutes
        .trim()
        .parse::<u32>()
        .map_err(|_| "Enter an interval between 5 and 10080 minutes.".to_owned())?;
    Ok(PortfolioSchedule::Interval {
        every_minutes,
        enabled,
    })
}

fn next_portfolio_revision_suggestion(current: &str) -> String {
    current
        .strip_prefix('v')
        .and_then(|value| value.parse::<u32>().ok())
        .and_then(|value| value.checked_add(1))
        .map_or_else(|| "candidate-v1".to_owned(), |value| format!("v{value}"))
}

fn next_portfolio_route_id(
    draft: Option<&PortfolioDraft>,
    source: &str,
    destination: &str,
) -> String {
    let existing = draft.map(|draft| &draft.orchestration().routes);
    (1_u32..=10_000)
        .map(|index| format!("route-{source}-{destination}-{index}"))
        .find(|candidate| {
            existing.is_none_or(|routes| routes.iter().all(|route| route.id != *candidate))
        })
        .unwrap_or_else(|| "route-next".to_owned())
}

fn portfolio_allowed_signals(kind: &PortfolioNodeKind) -> Vec<PortfolioSignal> {
    match kind {
        PortfolioNodeKind::ProjectSelector { .. } | PortfolioNodeKind::ProjectInvocation { .. } => {
            vec![
                PortfolioSignal::Completed,
                PortfolioSignal::NoCandidate,
                PortfolioSignal::NeedsAttention,
                PortfolioSignal::Failed,
                PortfolioSignal::Manual,
            ]
        }
        PortfolioNodeKind::PostAction {
            action: PortfolioPostAction::RecordSummary,
        } => vec![
            PortfolioSignal::Completed,
            PortfolioSignal::Failed,
            PortfolioSignal::Manual,
        ],
        PortfolioNodeKind::PostAction {
            action: PortfolioPostAction::RequestApproval,
        } => vec![PortfolioSignal::Approved, PortfolioSignal::Rejected],
        PortfolioNodeKind::Terminal => Vec::new(),
    }
}

const PORTFOLIO_COMPLETION_BRANCH_SIGNALS: [PortfolioSignal; 2] =
    [PortfolioSignal::Completed, PortfolioSignal::Failed];
const PORTFOLIO_SELECTOR_BRANCH_SIGNALS: [PortfolioSignal; 2] =
    [PortfolioSignal::Completed, PortfolioSignal::NoCandidate];
const PORTFOLIO_APPROVAL_BRANCH_SIGNALS: [PortfolioSignal; 2] =
    [PortfolioSignal::Approved, PortfolioSignal::Rejected];

fn primary_portfolio_branch_signals(kind: &PortfolioNodeKind) -> &'static [PortfolioSignal] {
    match kind {
        PortfolioNodeKind::ProjectSelector { .. } => &PORTFOLIO_SELECTOR_BRANCH_SIGNALS,
        PortfolioNodeKind::ProjectInvocation { .. }
        | PortfolioNodeKind::PostAction {
            action: PortfolioPostAction::RecordSummary,
        } => &PORTFOLIO_COMPLETION_BRANCH_SIGNALS,
        PortfolioNodeKind::PostAction {
            action: PortfolioPostAction::RequestApproval,
        } => &PORTFOLIO_APPROVAL_BRANCH_SIGNALS,
        PortfolioNodeKind::Terminal => &[],
    }
}

fn portfolio_signal_value(signal: PortfolioSignal) -> &'static str {
    signal.as_str()
}

#[component]
fn PortfolioOrchestrationCanvas(
    revision: PortfolioOrchestrationRevision,
    canvas_layout: Option<GraphCanvasLayout>,
    canvas_layout_id: String,
    selected_node_id: String,
    connect_signal: Option<PortfolioSignal>,
    on_node_select: EventHandler<String>,
    on_flow_connect: EventHandler<(String, String, PortfolioSignal)>,
    on_layout_save: EventHandler<GraphCanvasLayout>,
) -> Element {
    let initial_node_ids = revision
        .nodes
        .iter()
        .map(|node| node.id.clone())
        .collect::<Vec<_>>();
    let initial_positions = canvas_position_map(canvas_layout.as_ref(), &initial_node_ids);
    let mut dragged_node_id = use_signal(String::new);
    let mut dragged_route_source_id = use_signal(String::new);
    let mut dragged_route_signal = use_signal(|| None::<PortfolioSignal>);
    let mut drop_target_node_id = use_signal(String::new);
    let mut node_positions = use_signal(move || initial_positions);
    let mut drag_origin = use_signal(|| None::<CanvasPoint>);
    let mut drag_start_position = use_signal(|| None::<CanvasPoint>);
    let mut pointer_drag_active = use_signal(|| false);
    let mut drag_preview = use_signal(|| None::<(String, CanvasPoint)>);
    let drop_target_value = drop_target_node_id.read().clone();
    let dragged_node_value = dragged_node_id.read().clone();
    let pointer_drag_value = *pointer_drag_active.read();
    let node_ids = initial_node_ids;
    let entry_node_ids = vec![revision.entry_node_id.clone()];
    let routes = revision
        .routes
        .iter()
        .map(|route| {
            (
                route.source_node_id.clone(),
                route.destination_node_id.clone(),
            )
        })
        .collect::<Vec<_>>();
    let layout = ControlGraphLayout::from_topology(&node_ids, &entry_node_ids, &routes);
    let mut rendered_positions = node_positions.read().clone();
    if let Some((node_id, point)) = drag_preview.read().clone() {
        rendered_positions.insert(node_id, point);
    }
    let (stage_width, stage_height) = layout.freeform_stage_dimensions(&rendered_positions);
    let stage_style = format!("width: {stage_width}px; min-height: {stage_height}px;");
    let view_box = format!("0 0 {stage_width} {stage_height}");
    let arrange_layout_id = canvas_layout_id.clone();
    let move_layout_id = canvas_layout_id;
    rsx! {
        div { class: "portfolio-orchestration-canvas",
            div { class: "portfolio-canvas-toolbar",
                small { class: "blueprint-canvas-hint",
                    if let Some(signal) = connect_signal {
                        "Drag cards to move them · drag a labeled branch pin to connect a {portfolio_signal_label(signal)} or alternate route."
                    } else {
                        "Drag cards to move them. Labeled branch pins expose each outcome; Terminal nodes have none."
                    }
                }
                button {
                    disabled: node_positions.read().is_empty(),
                    title: "Return moved nodes to automatic layout",
                    onclick: move |_| {
                        node_positions.write().clear();
                        on_layout_save.call(graph_canvas_layout_from_positions(
                            &arrange_layout_id,
                            &node_positions.read(),
                        ));
                    },
                    "Auto arrange"
                }
            }
            div { class: "graph-layout-scroll",
                div {
                    class: "graph-stage graph-freeform-stage",
                    style: "{stage_style}",
                    onpointermove: move |event| {
                        let node_id = dragged_node_id.read().clone();
                        if node_id.is_empty() {
                            return;
                        }
                        let current = event.client_coordinates();
                        let current = CanvasPoint { x: current.x, y: current.y };
                        let origin = *drag_origin.read();
                        let exceeded = origin
                            .is_some_and(|origin| pointer_drag_exceeded(origin, current));
                        if exceeded || *pointer_drag_active.read() {
                            pointer_drag_active.set(true);
                            if let (Some(origin), Some(start_position)) =
                                (origin, *drag_start_position.read())
                            {
                                drag_preview.set(Some((
                                    node_id,
                                    graph_drag_preview_point(
                                        start_position,
                                        origin,
                                        current,
                                        1.0,
                                    ),
                                )));
                            }
                        }
                    },
                    onpointerup: move |_| {
                        let node_id = dragged_node_id.read().clone();
                        if !node_id.is_empty() && *pointer_drag_active.read() {
                            let final_point = drag_preview
                                .read()
                                .as_ref()
                                .filter(|(preview_node_id, _)| preview_node_id == &node_id)
                                .map(|(_, preview_point)| *preview_point)
                                .or(*drag_start_position.read());
                            if let Some(final_point) = final_point {
                                node_positions.write().insert(node_id, final_point);
                                on_layout_save.call(graph_canvas_layout_from_positions(
                                    &move_layout_id,
                                    &node_positions.read(),
                                ));
                            }
                        }
                        dragged_node_id.set(String::new());
                        dragged_route_source_id.set(String::new());
                        dragged_route_signal.set(None);
                        drop_target_node_id.set(String::new());
                        drag_origin.set(None);
                        drag_start_position.set(None);
                        pointer_drag_active.set(false);
                        drag_preview.set(None);
                    },
                    onpointercancel: move |_| {
                        dragged_node_id.set(String::new());
                        dragged_route_source_id.set(String::new());
                        dragged_route_signal.set(None);
                        drop_target_node_id.set(String::new());
                        drag_origin.set(None);
                        drag_start_position.set(None);
                        pointer_drag_active.set(false);
                        drag_preview.set(None);
                    },
                    svg {
                        class: "graph-route-layer",
                        view_box: "{view_box}",
                        width: "{stage_width}",
                        height: "{stage_height}",
                        defs {
                            marker {
                                id: "portfolio-route-arrow",
                                marker_width: "7",
                                marker_height: "7",
                                ref_x: "6",
                                ref_y: "3",
                                orient: "auto",
                                marker_units: "strokeWidth",
                                path { d: "M 0 0 L 6 3 L 0 6 z" }
                            }
                        }
                        for route in &revision.routes {
                            if let Some(path_data) = layout.freeform_branch_route_path(
                                &route.source_node_id,
                                &route.destination_node_id,
                                &rendered_positions,
                                portfolio_route_source_port_y(
                                    &revision,
                                    &route.source_node_id,
                                    route.signal,
                                    connect_signal,
                                ),
                                GRAPH_INPUT_PORT_Y,
                            ) {
                                path {
                                    key: "portfolio-path-{route.id}",
                                    class: format!(
                                        "graph-route-path signal-{}",
                                        portfolio_signal_value(route.signal),
                                    ),
                                    d: "{path_data}",
                                    marker_end: "url(#portfolio-route-arrow)",
                                }
                            }
                        }
                    }
                    div { class: "graph-freeform-nodes",
                        for node in &revision.nodes {
                            {
                                let can_start_route =
                                    !matches!(node.kind, PortfolioNodeKind::Terminal);
                                let point = rendered_positions
                                    .get(&node.id)
                                    .copied()
                                    .or_else(|| layout.canvas_point(&node.id))
                                    .unwrap_or_default();
                                let node_id = node.id.clone();
                                let allowed_signals = portfolio_allowed_signals(&node.kind);
                                let outgoing_signals = revision
                                    .routes
                                    .iter()
                                    .filter(|route| route.source_node_id == node.id)
                                    .map(|route| route.signal)
                                    .collect::<Vec<_>>();
                                let branch_signals = branch_signal_options(
                                    &allowed_signals,
                                    primary_portfolio_branch_signals(&node.kind),
                                    &outgoing_signals,
                                    connect_signal,
                                );
                                let node_style =
                                    graph_freeform_node_style(point, branch_signals.len());
                                rsx! {
                                    button {
                                        key: "portfolio-node-{node.id}",
                                        class: match (
                                            node.id == dragged_node_value && pointer_drag_value,
                                            node.id == drop_target_value,
                                            node.id == selected_node_id,
                                            node.id == revision.entry_node_id,
                                            layout.is_reachable(&node.id),
                                        ) {
                                            (true, _, _, _, _) => "graph-node graph-freeform-node portfolio-orchestration-node dragging",
                                            (_, true, _, _, _) => "graph-node graph-freeform-node portfolio-orchestration-node drop-target",
                                            (_, _, true, true, _) => "graph-node graph-freeform-node portfolio-orchestration-node entry active",
                                            (_, _, true, _, _) => "graph-node graph-freeform-node portfolio-orchestration-node active",
                                            (_, _, _, true, _) => "graph-node graph-freeform-node portfolio-orchestration-node entry",
                                            (_, _, _, _, false) => "graph-node graph-freeform-node portfolio-orchestration-node unreachable",
                                            _ => "graph-node graph-freeform-node portfolio-orchestration-node",
                                        },
                                        style: "{node_style}",
                                        title: "Drag the card to move it. Use the right socket to connect it.",
                                        onclick: move |_| on_node_select.call(node_id.clone()),
                                        onpointerdown: {
                                            let node_id = node.id.clone();
                                            let start_position = point;
                                            move |event| {
                                                if dragged_route_source_id.read().is_empty() {
                                                    let origin = event.client_coordinates();
                                                    dragged_node_id.set(node_id.clone());
                                                    dragged_route_signal.set(None);
                                                    drag_origin.set(Some(CanvasPoint { x: origin.x, y: origin.y }));
                                                    drag_start_position.set(Some(start_position));
                                                    pointer_drag_active.set(false);
                                                    drag_preview.set(None);
                                                    on_node_select.call(node_id.clone());
                                                }
                                            }
                                        },
                                        onpointerenter: {
                                            let destination = node.id.clone();
                                            move |_| {
                                                let source = dragged_route_source_id.read();
                                                if !source.is_empty() && source.as_str() != destination {
                                                    drop_target_node_id.set(destination.clone());
                                                }
                                            }
                                        },
                                        onpointerup: {
                                            let destination = node.id.clone();
                                            move |event| {
                                                let source = dragged_route_source_id.read().clone();
                                                let signal = *dragged_route_signal.read();
                                                if !source.is_empty() {
                                                    event.stop_propagation();
                                                    if source != destination
                                                        && let Some(signal) = signal
                                                    {
                                                        on_flow_connect.call((source, destination.clone(), signal));
                                                    }
                                                    dragged_route_source_id.set(String::new());
                                                    dragged_route_signal.set(None);
                                                    drop_target_node_id.set(String::new());
                                                }
                                            }
                                        },
                                        span { class: "graph-node-socket graph-node-socket-input", aria_hidden: "true" }
                                        if node.id == revision.entry_node_id {
                                            span { class: "graph-node-entry", "Scheduled entry" }
                                        } else if !layout.is_reachable(&node.id) {
                                            span { class: "graph-node-entry graph-node-unconnected", "Connect me" }
                                        }
                                        strong { "{node.id}" }
                                        small { "{portfolio_node_kind_label(&node.kind)}" }
                                        if can_start_route {
                                            div { class: "graph-node-branches", aria_label: "Branches from {node.id}",
                                                for (branch_index, signal) in branch_signals.into_iter().enumerate() {
                                                    {
                                                        let branch_style = graph_branch_pin_style(branch_index);
                                                        let destination = revision.routes.iter().find_map(|route| {
                                                            (route.source_node_id == node.id && route.signal == signal)
                                                                .then_some(route.destination_node_id.clone())
                                                        });
                                                        let branch_class = if destination.is_some() {
                                                            "graph-node-branch-pin connected"
                                                        } else {
                                                            "graph-node-branch-pin"
                                                        };
                                                        let can_connect = destination.is_none();
                                                        let signal_label = portfolio_signal_label(signal);
                                                        let branch_title = destination.as_ref().map_or_else(
                                                            || format!("Drag the {signal_label} branch to another node"),
                                                            |destination| format!("{signal_label} connects to {destination}. Remove it before reconnecting."),
                                                        );
                                                        rsx! {
                                                            span {
                                                                class: "{branch_class}",
                                                                style: "{branch_style}",
                                                                title: "{branch_title}",
                                                                aria_label: "Connect {signal_label} from {node.id}",
                                                                onpointerdown: {
                                                                    let node_id = node.id.clone();
                                                                    move |event| {
                                                                        event.stop_propagation();
                                                                        if !can_connect {
                                                                            return;
                                                                        }
                                                                        dragged_node_id.set(String::new());
                                                                        drag_origin.set(None);
                                                                        drag_start_position.set(None);
                                                                        pointer_drag_active.set(false);
                                                                        drag_preview.set(None);
                                                                        dragged_route_source_id.set(node_id.clone());
                                                                        dragged_route_signal.set(Some(signal));
                                                                    }
                                                                },
                                                                span { class: "graph-node-branch-label", "{signal_label}" }
                                                                span { class: "graph-node-socket graph-node-socket-output", aria_hidden: "true" }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            div { class: "portfolio-orchestration-routes",
                for route in &revision.routes {
                    span { key: "portfolio-route-{route.id}",
                        code { "{route.source_node_id}" }
                        small { "{portfolio_signal_label(route.signal)}" }
                        code { "{route.destination_node_id}" }
                    }
                }
            }
        }
    }
}

#[component]
fn PortfolioStepPreviewCard(preview: PortfolioStepPreview) -> Element {
    rsx! {
        article { class: "portfolio-step-preview",
            div { class: "portfolio-step-preview-heading",
                div {
                    span { "Next bounded tick" }
                    strong { "{preview.node_id}" }
                }
                code { "{preview.orchestration_id}/{preview.revision_id}" }
            }
            match &preview.outcome {
                PortfolioStepOutcome::ProjectSelection {
                    project_id,
                    graph_binding,
                    autopilot,
                } => rsx! {
                    if let (Some(project_id), Some(graph_binding)) = (project_id, graph_binding) {
                        p { "Selected {project_id} through {graph_binding.graph_id}/{graph_binding.revision_id} · entry {graph_binding.entry_id}." }
                    } else {
                        p { "Project Selector inspected its managed project set." }
                    }
                    match &autopilot.outcome {
                        SafeAutopilotOutcome::Candidate(candidate) => rsx! {
                            div { class: "portfolio-step-target",
                                strong { "{candidate.work_item.id} · {candidate.work_item.title}" }
                                span { "{candidate.project_name} · {candidate.agent_role} ({candidate.agent_profile_id})" }
                            }
                        },
                        SafeAutopilotOutcome::NoCandidate(reason) => rsx! {
                            p { class: "portfolio-step-note", "{no_candidate_message(*reason)}" }
                        },
                        SafeAutopilotOutcome::Stop(reason) => rsx! {
                            p { class: "portfolio-step-warning", "{autopilot_stop_message(reason)}" }
                        },
                    }
                },
                PortfolioStepOutcome::ProjectInvocation {
                    project_id,
                    graph_binding,
                    autopilot,
                } => rsx! {
                    p { "Inspect {project_id} through {graph_binding.graph_id}/{graph_binding.revision_id} · entry {graph_binding.entry_id}." }
                    match &autopilot.outcome {
                        SafeAutopilotOutcome::Candidate(candidate) => rsx! {
                            div { class: "portfolio-step-target",
                                strong { "{candidate.work_item.id} · {candidate.work_item.title}" }
                                span { "Agent · {candidate.agent_role} ({candidate.agent_profile_id})" }
                            }
                        },
                        SafeAutopilotOutcome::NoCandidate(reason) => rsx! {
                            p { class: "portfolio-step-note", "{no_candidate_message(*reason)}" }
                        },
                        SafeAutopilotOutcome::Stop(reason) => rsx! {
                            p { class: "portfolio-step-warning", "{autopilot_stop_message(reason)}" }
                        },
                    }
                },
                PortfolioStepOutcome::PostAction { action } => rsx! {
                    p { "Post action ready · {portfolio_post_action_label(*action)}" }
                },
                PortfolioStepOutcome::Terminal => rsx! {
                    p { "This orchestration revision is already at its terminal entry." }
                },
                PortfolioStepOutcome::Blocked { reason } => rsx! {
                    p { class: "portfolio-step-warning", "{portfolio_preview_blocked_message(reason)}" }
                },
            }
            small { "Preview only · no Runner or Portfolio Run mutation was started." }
        }
    }
}

fn portfolio_schedule_label(schedule: PortfolioSchedule) -> String {
    match schedule {
        PortfolioSchedule::Manual => "Manual only".to_owned(),
        PortfolioSchedule::Interval {
            every_minutes,
            enabled,
        } => format!(
            "Every {every_minutes} min · {}",
            if enabled { "configured" } else { "disabled" }
        ),
    }
}

fn portfolio_node_kind_label(kind: &PortfolioNodeKind) -> String {
    match kind {
        PortfolioNodeKind::ProjectSelector { selector } => match selector {
            PortfolioProjectSelector::AllManaged => "Project Selector · All managed".to_owned(),
            PortfolioProjectSelector::Include { project_ids } => {
                format!("Project Selector · {} selected", project_ids.len())
            }
        },
        PortfolioNodeKind::ProjectInvocation { project_id } => {
            format!("Project · {project_id}")
        }
        PortfolioNodeKind::PostAction { action } => {
            format!("Post action · {}", portfolio_post_action_label(*action))
        }
        PortfolioNodeKind::Terminal => "Terminal".to_owned(),
    }
}

fn portfolio_post_action_label(action: PortfolioPostAction) -> &'static str {
    match action {
        PortfolioPostAction::RecordSummary => "Record summary",
        PortfolioPostAction::RequestApproval => "Request approval",
    }
}

fn portfolio_signal_label(signal: PortfolioSignal) -> &'static str {
    match signal {
        PortfolioSignal::Completed => "Completed",
        PortfolioSignal::NoCandidate => "No candidate",
        PortfolioSignal::NeedsAttention => "Needs attention",
        PortfolioSignal::Failed => "Failed",
        PortfolioSignal::Approved => "Approved",
        PortfolioSignal::Rejected => "Rejected",
        PortfolioSignal::Manual => "Manual",
    }
}

fn portfolio_preview_blocked_message(reason: &PortfolioPreviewBlockedReason) -> String {
    match reason {
        PortfolioPreviewBlockedReason::InvalidRevision(_) => {
            "The Portfolio orchestration revision is invalid.".to_owned()
        }
        PortfolioPreviewBlockedReason::NodeNotFound { node_id } => {
            format!("Portfolio node {node_id} is unavailable in the pinned revision.")
        }
        PortfolioPreviewBlockedReason::ProjectNotFound { project_id } => {
            format!("Managed project {project_id} no longer exists.")
        }
        PortfolioPreviewBlockedReason::ProjectGraphBindingNotFound { project_id } => format!(
            "Select a Control Graph for {project_id} in All project graphs before scheduling it."
        ),
        PortfolioPreviewBlockedReason::GraphRevisionNotFound {
            graph_id,
            revision_id,
        } => format!("Control Graph {graph_id}/{revision_id} is unavailable."),
        PortfolioPreviewBlockedReason::InvalidProjectGraphBinding { project_id } => {
            format!("The Control Graph selection for {project_id} is invalid.")
        }
    }
}

fn portfolio_run_status_label(status: PortfolioRunStatus) -> &'static str {
    match status {
        PortfolioRunStatus::Active => "Active",
        PortfolioRunStatus::WaitingApproval => "Waiting for approval",
        PortfolioRunStatus::Paused => "Paused safely",
        PortfolioRunStatus::Completed => "Completed",
    }
}

#[component]
fn PortfolioGraphManager(
    portfolio: PortfolioSnapshot,
    control_graphs: Vec<ControlGraphRevision>,
    graph_canvas_layouts: Vec<GraphCanvasLayout>,
    agent_profiles: Vec<AgentProfileSummary>,
    project_graph_bindings: Vec<ProjectGraphBinding>,
    graph_rewrite_proposals: Vec<GraphRewriteProposal>,
    work_item_graph_positions: Vec<WorkItemGraphPosition>,
    route_decisions: Vec<RouteDecision>,
    on_graph_select: EventHandler<ProjectGraphBindingSaveRequest>,
    on_graph_canvas_layout: EventHandler<GraphCanvasLayout>,
    on_graph_rewrite_propose: EventHandler<GraphRewriteProposalRequest>,
    on_graph_rewrite_decide: EventHandler<GraphRewriteDecisionRequest>,
    on_evidence_route: EventHandler<EvidenceRouteRequest>,
    on_human_approval: EventHandler<HumanApprovalRequest>,
) -> Element {
    let project_ids = portfolio
        .projects
        .iter()
        .map(|project| project.id.clone())
        .collect::<Vec<_>>();
    let initial_project_id = project_ids.first().cloned().unwrap_or_default();
    let mut selected_project_id = use_signal(move || initial_project_id);
    let requested_project_id = selected_project_id.read().clone();
    let active_project_id = resolve_managed_project_id(&project_ids, &requested_project_id)
        .map_or_else(String::new, ToOwned::to_owned);
    let active_project = portfolio
        .projects
        .iter()
        .find(|project| project.id == active_project_id);

    rsx! {
        section { class: "section-heading portfolio-graph-heading",
            div {
                p { class: "kicker", "Graph engineering" }
                h3 { "All project graphs" }
            }
            span { "{portfolio.projects.len()} managed" }
        }
        section { class: "portfolio-graph-manager", aria_label: "All managed project graphs",
            header { class: "portfolio-graph-intro",
                div {
                    strong { "Choose any managed project" }
                    p { "Bind its live Graph revision, edit the draft map, and send changes through the same approval path." }
                }
                span { "One workspace · every project" }
            }
            if portfolio.projects.is_empty() {
                p { class: "portfolio-graph-empty", "Add a project to start managing its Control Graph." }
            } else {
                nav { class: "portfolio-graph-projects", aria_label: "Managed projects",
                    for project in &portfolio.projects {
                        {
                            let project_id = project.id.clone();
                            let binding = project_graph_bindings
                                .iter()
                                .find(|binding| binding.project_id == project.id);
                            rsx! {
                                button {
                                    key: "graph-project-{project.id}",
                                    class: if project.id == active_project_id {
                                        "portfolio-graph-project active"
                                    } else {
                                        "portfolio-graph-project"
                                    },
                                    aria_pressed: project.id == active_project_id,
                                    onclick: move |_| selected_project_id.set(project_id.clone()),
                                    strong { "{project.name}" }
                                    small { "{project.id}" }
                                    span { "{project_graph_binding_label(binding)}" }
                                }
                            }
                        }
                    }
                }
                if let Some(project) = active_project {
                    div { class: "portfolio-graph-workspace",
                        header {
                            div {
                                p { class: "project-id", "{project.id}" }
                                h4 { "{project.name}" }
                            }
                            span { class: health_class(project.health), "{project.health.label()}" }
                        }
                        ProjectGraphWorkspace {
                            key: "portfolio-graph-{project.id}",
                            project_id: project.id.clone(),
                            control_graphs: control_graphs.clone(),
                            graph_canvas_layouts: graph_canvas_layouts.clone(),
                            agent_profiles: agent_profiles.clone(),
                            graph_binding: project_graph_bindings
                                .iter()
                                .find(|binding| binding.project_id == project.id)
                                .cloned(),
                            graph_rewrite_proposals: graph_rewrite_proposals
                                .iter()
                                .filter(|proposal| proposal.project_id == project.id)
                                .cloned()
                                .collect(),
                            graph_positions: work_item_graph_positions
                                .iter()
                                .filter(|position| position.project_id == project.id)
                                .cloned()
                                .collect(),
                            route_decisions: route_decisions
                                .iter()
                                .filter(|decision| decision.project_id == project.id)
                                .cloned()
                                .collect(),
                            on_graph_select,
                            on_graph_canvas_layout,
                            on_graph_rewrite_propose,
                            on_graph_rewrite_decide,
                            on_evidence_route,
                            on_human_approval,
                        }
                    }
                }
            }
        }
    }
}

fn resolve_managed_project_id<'a>(project_ids: &'a [String], requested: &str) -> Option<&'a str> {
    project_ids
        .iter()
        .find(|project_id| project_id.as_str() == requested)
        .or_else(|| project_ids.first())
        .map(String::as_str)
}

fn project_graph_binding_label(binding: Option<&ProjectGraphBinding>) -> String {
    binding.map_or_else(
        || "Graph not selected".to_owned(),
        |binding| format!("{} / {}", binding.graph_id, binding.revision_id),
    )
}

#[component]
fn ProjectGrid(
    portfolio: PortfolioSnapshot,
    execution_workspaces: Vec<ExecutionWorkspaceConnection>,
    control_graphs: Vec<ControlGraphRevision>,
    graph_canvas_layouts: Vec<GraphCanvasLayout>,
    agent_profiles: Vec<AgentProfileSummary>,
    project_graph_bindings: Vec<ProjectGraphBinding>,
    graph_rewrite_proposals: Vec<GraphRewriteProposal>,
    work_item_graph_positions: Vec<WorkItemGraphPosition>,
    route_decisions: Vec<RouteDecision>,
    on_connect: EventHandler<(String, Option<ExecutionWorkspaceConnection>, String)>,
    on_graph_select: EventHandler<ProjectGraphBindingSaveRequest>,
    on_graph_canvas_layout: EventHandler<GraphCanvasLayout>,
    on_graph_rewrite_propose: EventHandler<GraphRewriteProposalRequest>,
    on_graph_rewrite_decide: EventHandler<GraphRewriteDecisionRequest>,
    on_evidence_route: EventHandler<EvidenceRouteRequest>,
    on_human_approval: EventHandler<HumanApprovalRequest>,
    new_project_id: Signal<String>,
    new_project_name: Signal<String>,
    new_project_execution_cap: Signal<String>,
    new_project_workspace_location: Signal<String>,
    on_create: EventHandler<(String, String, u32, String)>,
) -> Element {
    let project_count = portfolio.projects.len();
    let existing_project_ids = portfolio
        .projects
        .iter()
        .map(|project| project.id.clone())
        .collect::<Vec<_>>();
    rsx! {
        section { class: "section-heading",
            div {
                p { class: "kicker", "Connected work" }
                h3 { "Projects" }
            }
            span { "{project_count} total" }
        }

        NewProjectForm {
            existing_project_ids,
            project_id: new_project_id,
            name: new_project_name,
            execution_cap: new_project_execution_cap,
            workspace_location: new_project_workspace_location,
            on_create,
        }

        section { class: "project-grid",
            for project in &portfolio.projects {
                ProjectCard {
                    key: "{project.id}",
                    project: project.clone(),
                    execution_workspace: execution_workspaces
                        .iter()
                        .find(|connection| connection.project_id == project.id)
                        .cloned(),
                    control_graphs: control_graphs.clone(),
                    graph_canvas_layouts: graph_canvas_layouts.clone(),
                    agent_profiles: agent_profiles.clone(),
                    graph_binding: project_graph_bindings
                        .iter()
                        .find(|binding| binding.project_id == project.id)
                        .cloned(),
                    graph_positions: work_item_graph_positions
                        .iter()
                        .filter(|position| position.project_id == project.id)
                        .cloned()
                        .collect(),
                    graph_rewrite_proposals: graph_rewrite_proposals
                        .iter()
                        .filter(|proposal| proposal.project_id == project.id)
                        .cloned()
                        .collect(),
                    route_decisions: route_decisions
                        .iter()
                        .filter(|decision| decision.project_id == project.id)
                        .cloned()
                        .collect(),
                    on_connect,
                    on_graph_select,
                    on_graph_canvas_layout,
                    on_graph_rewrite_propose,
                    on_graph_rewrite_decide,
                    on_evidence_route,
                    on_human_approval,
                }
            }
        }
    }
}

#[component]
fn NewProjectForm(
    existing_project_ids: Vec<String>,
    mut project_id: Signal<String>,
    mut name: Signal<String>,
    mut execution_cap: Signal<String>,
    mut workspace_location: Signal<String>,
    on_create: EventHandler<(String, String, u32, String)>,
) -> Element {
    let project_id_value = project_id.read().clone();
    let name_value = name.read().clone();
    let execution_cap_value = execution_cap.read().clone();
    let workspace_location_value = workspace_location.read().clone();
    let parsed_execution_cap = parse_project_execution_cap(&execution_cap_value);
    let duplicate_id = existing_project_ids
        .iter()
        .any(|existing_project_id| existing_project_id == &project_id_value);
    let can_create = project_id_is_valid(&project_id_value)
        && project_name_is_valid(&name_value)
        && parsed_execution_cap.is_some()
        && !workspace_location_value.trim().is_empty()
        && !duplicate_id;
    rsx! {
        details { class: "new-project",
            summary { "Add existing project" }
            div { class: "project-form",
                label {
                    span { "Stable ID" }
                    input {
                        aria_label: "New project ID",
                        value: "{project_id_value}",
                        maxlength: 64,
                        placeholder: "my-product",
                        oninput: move |event| project_id.set(event.value()),
                    }
                    small { "Lowercase letters, numbers, hyphens, or underscores." }
                }
                label {
                    span { "Project name" }
                    input {
                        aria_label: "New project name",
                        value: "{name_value}",
                        maxlength: 256,
                        placeholder: "My product",
                        oninput: move |event| name.set(event.value()),
                    }
                }
                label {
                    span { "Execution capacity" }
                    input {
                        aria_label: "New project execution capacity",
                        r#type: "number",
                        min: "1",
                        max: "4294967295",
                        value: "{execution_cap_value}",
                        oninput: move |event| execution_cap.set(event.value()),
                    }
                    small { "Positive local scheduling capacity; this does not start a Runner." }
                }
                label {
                    span { "Existing local directory" }
                    input {
                        aria_label: "New project execution workspace",
                        value: "{workspace_location_value}",
                        maxlength: 2048,
                        placeholder: "Select an existing project directory",
                        oninput: move |event| workspace_location.set(event.value()),
                    }
                    small { "Stored as an in-place local connection only." }
                }
                if duplicate_id {
                    p { class: "field-warning", "That Board project ID already exists." }
                }
                button {
                    class: "project-create-action",
                    disabled: !can_create,
                    onclick: move |_| {
                        if let Some(execution_cap) = parsed_execution_cap {
                            on_create.call((
                                project_id_value.clone(),
                                name_value.trim().to_owned(),
                                execution_cap,
                                workspace_location_value.clone(),
                            ));
                        }
                    },
                    "Add project"
                }
            }
        }
    }
}

#[component]
fn ProjectGraphWorkspace(
    project_id: String,
    control_graphs: Vec<ControlGraphRevision>,
    graph_canvas_layouts: Vec<GraphCanvasLayout>,
    agent_profiles: Vec<AgentProfileSummary>,
    graph_binding: Option<ProjectGraphBinding>,
    graph_rewrite_proposals: Vec<GraphRewriteProposal>,
    graph_positions: Vec<WorkItemGraphPosition>,
    route_decisions: Vec<RouteDecision>,
    on_graph_select: EventHandler<ProjectGraphBindingSaveRequest>,
    on_graph_canvas_layout: EventHandler<GraphCanvasLayout>,
    on_graph_rewrite_propose: EventHandler<GraphRewriteProposalRequest>,
    on_graph_rewrite_decide: EventHandler<GraphRewriteDecisionRequest>,
    on_evidence_route: EventHandler<EvidenceRouteRequest>,
    on_human_approval: EventHandler<HumanApprovalRequest>,
) -> Element {
    let graph_project_id = project_id.clone();
    let graph_expected = graph_binding.clone();
    let selected_graph_value = graph_binding.as_ref().map_or_else(String::new, |binding| {
        format!(
            "{}|{}|{}",
            binding.graph_id, binding.revision_id, binding.entry_id
        )
    });
    let selected_graph = graph_binding.as_ref().and_then(|binding| {
        control_graphs.iter().find(|graph| {
            graph.graph_id == binding.graph_id && graph.revision_id == binding.revision_id
        })
    });
    let graph_label = graph_binding.as_ref().map_or_else(
        || "Not selected".to_owned(),
        |binding| {
            format!(
                "{} / {} · {}",
                binding.graph_id, binding.revision_id, binding.entry_id
            )
        },
    );
    let graph_options = control_graphs
        .iter()
        .flat_map(|graph| {
            graph.entries.iter().map(|entry| {
                (
                    format!("{}|{}|{}", graph.graph_id, graph.revision_id, entry.id),
                    format!("{} / {} · {}", graph.graph_id, graph.revision_id, entry.id),
                    graph.graph_id.clone(),
                    graph.revision_id.clone(),
                    entry.id.clone(),
                )
            })
        })
        .collect::<Vec<_>>();
    rsx! {
        div { class: "control-graph-connection",
            div { class: "control-graph-heading",
                div {
                    span { "Control graph" }
                    strong { "{graph_label}" }
                }
                span { class: "graph-revision-badge", "Version pinned" }
            }
            label {
                span { "Default route entry" }
                select {
                    aria_label: "Control graph for {project_id}",
                    value: "{selected_graph_value}",
                    onchange: move |event| {
                        let value = event.value();
                        if let Some(option) = graph_options
                            .iter()
                            .find(|option| option.0 == value)
                        {
                            on_graph_select.call(ProjectGraphBindingSaveRequest {
                                expected: graph_expected.clone(),
                                target: ProjectGraphBinding {
                                    project_id: graph_project_id.clone(),
                                    graph_id: option.2.clone(),
                                    revision_id: option.3.clone(),
                                    entry_id: option.4.clone(),
                                },
                            });
                        }
                    },
                    option {
                        value: "",
                        disabled: true,
                        selected: graph_binding.is_none(),
                        "Select graph and entry"
                    }
                    for option in &graph_options {
                        option {
                            value: "{option.0}",
                            selected: option.0 == selected_graph_value,
                            "{option.1}"
                        }
                    }
                }
            }
            if let Some(graph) = selected_graph {
                dl { class: "control-graph-summary",
                    div {
                        dt { "Stages" }
                        dd { "{graph.nodes.len()}" }
                    }
                    div {
                        dt { "Routes" }
                        dd { "{graph.routes.len()}" }
                    }
                    div {
                        dt { "Anchors" }
                        dd { "{graph.anchors.len()}" }
                    }
                }
                small { "Autopilot may select only declared routes. Graph rewrites create a new revision." }
            } else {
                small { "Choose the immutable graph revision and entry Autopilot should use for future work." }
            }
            if let Some(selected_graph) = selected_graph {
                GraphRewritePanel {
                    key: "{selected_graph.revision_id}",
                    project_id: project_id.clone(),
                    graph_binding: graph_binding.clone(),
                    graph: Some(selected_graph.clone()),
                    canvas_layout: graph_canvas_layouts
                        .iter()
                        .find(|layout| layout.graph_id == selected_graph.graph_id)
                        .cloned(),
                    agent_profiles: agent_profiles.clone(),
                    proposals: graph_rewrite_proposals,
                    on_propose: on_graph_rewrite_propose,
                    on_decide: on_graph_rewrite_decide,
                    on_layout_save: on_graph_canvas_layout,
                }
            } else {
                details { class: "graph-rewrite-panel",
                    summary { "Graph builder · 0 approval request(s)" }
                    p { "Select a Control graph to open the visual builder." }
                }
            }
            if !graph_positions.is_empty() {
                div { class: "control-graph-positions",
                    span { "Active graph positions" }
                    for position in graph_positions {
                        ControlGraphPositionCard {
                            key: "{position.work_item_id}-{position.current_node_id}",
                            position: position.clone(),
                            graph: control_graphs
                                .iter()
                                .find(|graph| {
                                    graph.graph_id == position.graph_id
                                        && graph.revision_id == position.revision_id
                                })
                                .cloned(),
                            on_evidence_route,
                            on_human_approval,
                        }
                    }
                }
            }
            if !route_decisions.is_empty() {
                details { class: "route-decision-history",
                    summary { "Route decision history · {route_decisions.len()}" }
                    ol {
                        for (index, decision) in route_decisions.iter().enumerate() {
                            li { key: "{decision.decision_id}",
                                div { class: "route-decision-heading",
                                    span { "#{index + 1} · {decision.work_item_id}" }
                                    strong { "{control_signal_label(decision.signal)}" }
                                }
                                div { class: "route-decision-path",
                                    code { "{decision.source_node_id}" }
                                    span { "→" }
                                    code { "{decision.next_node_id}" }
                                }
                                p { "Route · {decision.route_id}" }
                                div { class: "route-decision-evidence",
                                    span { "Evidence" }
                                    for evidence_ref in &decision.evidence_refs {
                                        code { "{evidence_ref}" }
                                    }
                                }
                            }
                        }
                    }
                    small { "Immutable Board history in recorded order." }
                }
            }
        }
    }
}

#[component]
fn ProjectCard(
    project: gareji_board_domain::ProjectSummary,
    execution_workspace: Option<ExecutionWorkspaceConnection>,
    control_graphs: Vec<ControlGraphRevision>,
    graph_canvas_layouts: Vec<GraphCanvasLayout>,
    agent_profiles: Vec<AgentProfileSummary>,
    graph_binding: Option<ProjectGraphBinding>,
    graph_rewrite_proposals: Vec<GraphRewriteProposal>,
    graph_positions: Vec<WorkItemGraphPosition>,
    route_decisions: Vec<RouteDecision>,
    on_connect: EventHandler<(String, Option<ExecutionWorkspaceConnection>, String)>,
    on_graph_select: EventHandler<ProjectGraphBindingSaveRequest>,
    on_graph_canvas_layout: EventHandler<GraphCanvasLayout>,
    on_graph_rewrite_propose: EventHandler<GraphRewriteProposalRequest>,
    on_graph_rewrite_decide: EventHandler<GraphRewriteDecisionRequest>,
    on_evidence_route: EventHandler<EvidenceRouteRequest>,
    on_human_approval: EventHandler<HumanApprovalRequest>,
) -> Element {
    let initial_location = execution_workspace
        .as_ref()
        .and_then(|connection| connection.location.clone())
        .unwrap_or_default();
    let mut workspace_location = use_signal(move || initial_location);
    let workspace_location_value = workspace_location.read().clone();
    let connection_label = execution_workspace_label(execution_workspace.as_ref());
    let workspace_inspection =
        ExecutionWorkspaceConnector::inspect_connection(execution_workspace.as_ref());
    let availability_label = workspace_availability_label(workspace_inspection.availability);
    let availability_class = workspace_availability_class(workspace_inspection.availability);
    let repository_label = workspace_repository_label(&workspace_inspection);
    let workspace_instruction_files = workspace_inspection.workspace_instruction_files.clone();
    let instruction_discovery_note = workspace_instruction_discovery_note(
        workspace_inspection.availability,
        workspace_instruction_files.is_empty(),
    );
    let discovered_skill_ids = workspace_inspection.discovered_skill_ids.clone();
    let skill_discovery_note = workspace_skill_discovery_note(
        workspace_inspection.availability,
        discovered_skill_ids.is_empty(),
    );
    let can_connect = !workspace_location_value.trim().is_empty();
    let project_id = project.id.clone();
    let expected = execution_workspace.clone();
    rsx! {
        article { class: "project-card",
            div { class: "project-head",
                div {
                    p { class: "project-id", "{project.id}" }
                    h4 { "{project.name}" }
                }
                span { class: health_class(project.health), "{project.health.label()}" }
            }
            div { class: "project-stats",
                ProjectStat { label: "Todo", value: project.work_items.todo }
                ProjectStat { label: "Running", value: project.work_items.in_progress }
                ProjectStat { label: "Review", value: project.work_items.in_review }
                ProjectStat { label: "Blocked", value: project.work_items.blocked }
            }
            details { class: "project-detail-panel",
                summary {
                    div {
                        strong { "Control graph" }
                        small { "Binding, live position, routes, and safe draft edits" }
                    }
                    span { "Open" }
                }
                div { class: "project-detail-content",
                    ProjectGraphWorkspace {
                        project_id: project.id.clone(),
                        control_graphs: control_graphs.clone(),
                        graph_canvas_layouts,
                        agent_profiles,
                        graph_binding,
                        graph_rewrite_proposals,
                        graph_positions,
                        route_decisions,
                        on_graph_select,
                        on_graph_canvas_layout,
                        on_graph_rewrite_propose,
                        on_graph_rewrite_decide,
                        on_evidence_route,
                        on_human_approval,
                    }
                }
            }
            details { class: "project-detail-panel",
                summary {
                    div {
                        strong { "Execution workspace" }
                        small { "{connection_label}" }
                    }
                    span { "Open" }
                }
                div { class: "execution-workspace-connection project-detail-content",
                    div {
                        span { "Execution workspace" }
                        strong { "{connection_label}" }
                    }
                    dl { class: "workspace-connection-details",
                        div {
                            dt { "Connection" }
                            dd { class: "{availability_class}", "{availability_label}" }
                        }
                        div {
                            dt { "Repository" }
                            dd { "{repository_label}" }
                        }
                    }
                    div { class: "workspace-instructions",
                        span { "Workspace instructions" }
                        if workspace_instruction_files.is_empty() {
                            p { "{instruction_discovery_note}" }
                        } else {
                            ul {
                                for instruction_file in workspace_instruction_files {
                                    li { code { "{instruction_file}" } }
                                }
                            }
                            small { "Presence only; instructions are not loaded or applied." }
                        }
                    }
                    div { class: "workspace-discovered-skills",
                        span { "Discovered Skills" }
                        if discovered_skill_ids.is_empty() {
                            p { "{skill_discovery_note}" }
                        } else {
                            ul {
                                for skill_id in discovered_skill_ids {
                                    li { code { "{skill_id}" } }
                                }
                            }
                            small { "Detected only; enabling and trust remain separate." }
                        }
                    }
                    label {
                        span { "Connect existing local directory" }
                        input {
                            aria_label: "Execution workspace for {project.id}",
                            value: "{workspace_location_value}",
                            maxlength: 2048,
                            placeholder: "Select an existing project directory",
                            oninput: move |event| workspace_location.set(event.value()),
                        }
                    }
                    button {
                        class: "workspace-connect-action",
                        disabled: !can_connect,
                        onclick: move |_| {
                            on_connect.call((
                                project_id.clone(),
                                expected.clone(),
                                workspace_location_value.clone(),
                            ));
                        },
                        "Connect directory"
                    }
                    small { "Stores a local connection only. It does not copy files or enable execution." }
                }
            }
            footer {
                span { "Concurrency {project.work_items.in_progress}/{project.execution_cap}" }
                span { "{project.work_items.total} work items" }
            }
        }
    }
}

fn graph_canvas_position_map(
    layout: Option<&GraphCanvasLayout>,
    graph: Option<&ControlGraphRevision>,
) -> HashMap<String, CanvasPoint> {
    let Some(graph) = graph else {
        return HashMap::new();
    };
    let node_ids = graph
        .nodes
        .iter()
        .map(|node| node.id.clone())
        .collect::<Vec<_>>();
    canvas_position_map(layout, &node_ids)
}

fn canvas_position_map(
    layout: Option<&GraphCanvasLayout>,
    node_ids: &[String],
) -> HashMap<String, CanvasPoint> {
    let Some(layout) = layout else {
        return HashMap::new();
    };
    layout
        .positions
        .iter()
        .filter(|position| node_ids.iter().any(|node_id| node_id == &position.node_id))
        .map(|position| {
            (
                position.node_id.clone(),
                CanvasPoint {
                    x: position.x,
                    y: position.y,
                },
            )
        })
        .collect()
}

fn graph_canvas_layout_from_positions(
    graph_id: &str,
    positions: &HashMap<String, CanvasPoint>,
) -> GraphCanvasLayout {
    let mut positions = positions
        .iter()
        .map(|(node_id, point)| GraphCanvasNodePosition {
            node_id: node_id.clone(),
            x: point.x,
            y: point.y,
        })
        .collect::<Vec<_>>();
    positions.sort_by(|left, right| left.node_id.cmp(&right.node_id));
    GraphCanvasLayout {
        graph_id: graph_id.to_owned(),
        positions,
    }
}

#[component]
fn GraphRewritePanel(
    project_id: String,
    graph_binding: Option<ProjectGraphBinding>,
    graph: Option<ControlGraphRevision>,
    canvas_layout: Option<GraphCanvasLayout>,
    agent_profiles: Vec<AgentProfileSummary>,
    proposals: Vec<GraphRewriteProposal>,
    on_propose: EventHandler<GraphRewriteProposalRequest>,
    on_decide: EventHandler<GraphRewriteDecisionRequest>,
    on_layout_save: EventHandler<GraphCanvasLayout>,
) -> Element {
    let initial_graph = graph.clone();
    let layout_graph_id = graph
        .as_ref()
        .map_or_else(String::new, |graph| graph.graph_id.clone());
    let initial_node_positions = graph_canvas_position_map(canvas_layout.as_ref(), graph.as_ref());
    let initial_node_id = graph
        .as_ref()
        .and_then(|graph| graph.entries.first())
        .map_or_else(String::new, |entry| entry.node_id.clone());
    let mut draft = use_signal(move || initial_graph.and_then(|graph| GraphDraft::new(graph).ok()));
    let mut node_positions = use_signal(move || initial_node_positions);
    let mut edit_history = use_signal(Vec::<GraphEditorAction>::new);
    let mut selected_node_id = use_signal(move || initial_node_id);
    let mut replacement_agent_profile_id = use_signal(String::new);
    let mut new_node_id = use_signal(String::new);
    let mut new_node_kind = use_signal(|| "agent_loop".to_owned());
    let mut new_node_profile_id = use_signal(String::new);
    let mut connection_target_id = use_signal(String::new);
    let mut connection_signal =
        use_signal(|| control_signal_value(ControlSignal::Succeeded).to_owned());
    let mut rationale = use_signal(String::new);
    let mut evidence_ref = use_signal(String::new);
    let mut editor_notice = use_signal(String::new);
    let node_id_value = selected_node_id.read().clone();
    let replacement_value = replacement_agent_profile_id.read().clone();
    let new_node_id_value = new_node_id.read().clone();
    let new_node_kind_value = new_node_kind.read().clone();
    let new_node_profile_value = new_node_profile_id.read().clone();
    let connection_target_value = connection_target_id.read().clone();
    let connection_signal_value = connection_signal.read().clone();
    let rationale_value = rationale.read().clone();
    let evidence_ref_value = evidence_ref.read().clone();
    let editor_notice_value = editor_notice.read().clone();
    let draft_graph = draft.read().as_ref().map(|draft| draft.graph().clone());
    let selected_node = draft_graph.as_ref().and_then(|graph| {
        graph
            .nodes
            .iter()
            .find(|node| node.id == node_id_value)
            .cloned()
    });
    let current_agent_profile_id = selected_node.as_ref().and_then(|node| match &node.kind {
        ControlNodeKind::AgentLoop { agent_profile_id } => Some(agent_profile_id.clone()),
        _ => None,
    });
    let selected_is_entry = draft_graph.as_ref().is_some_and(|graph| {
        graph
            .entries
            .iter()
            .any(|entry| entry.node_id == node_id_value)
    });
    let topology_changed = draft.read().as_ref().is_some_and(GraphDraft::has_changes);
    let topology_is_valid = topology_changed
        && draft
            .read()
            .as_ref()
            .is_some_and(|draft| draft.revision("preview").is_ok());
    let profile_change_is_valid = !topology_changed
        && current_agent_profile_id.is_some()
        && !replacement_value.is_empty()
        && current_agent_profile_id.as_deref() != Some(replacement_value.as_str());
    let can_propose = graph_binding.is_some()
        && (topology_is_valid || profile_change_is_valid)
        && !rationale_value.trim().is_empty()
        && !evidence_ref_value.trim().is_empty();
    let connect_source_node_id = node_id_value.clone();
    let removable_node_id = node_id_value.clone();
    let proposal_node_id = node_id_value.clone();
    let allowed_connection_signals =
        control_signals_for_node(selected_node.as_ref().map(|node| &node.kind));
    let selected_connection_signal = parse_control_signal(&connection_signal_value)
        .filter(|signal| allowed_connection_signals.contains(signal))
        .or_else(|| allowed_connection_signals.first().copied())
        .unwrap_or(ControlSignal::Manual);
    let can_add_node = !new_node_id_value.trim().is_empty()
        && (new_node_kind_value != "agent_loop" || !new_node_profile_value.is_empty());
    let node_position_values = node_positions.read().clone();
    let can_undo = !edit_history.read().is_empty();
    let palette_agent_profile_id = new_node_profile_value.clone();
    let undo_layout_graph_id = layout_graph_id.clone();
    let reset_layout_graph_id = layout_graph_id.clone();
    let arrange_layout_graph_id = layout_graph_id.clone();
    let move_layout_graph_id = layout_graph_id.clone();
    let spawn_layout_graph_id = layout_graph_id.clone();
    let remove_layout_graph_id = layout_graph_id;
    rsx! {
        details { class: "graph-rewrite-panel", open: true,
            summary { "Graph builder · {proposals.len()} approval request(s)" }
            p { "Select nodes on the map, then add, connect, or remove them. Your live workflow stays unchanged until approval." }
            if let Some(binding) = graph_binding {
                if let Some(graph) = draft_graph.clone() {
                    GraphCanvas {
                        graph,
                        node_positions: node_position_values.clone(),
                        selected_node_id: node_id_value.clone(),
                        connection_signal: selected_connection_signal,
                        can_undo,
                        can_reset: topology_changed || !node_position_values.is_empty(),
                        can_auto_arrange: !node_position_values.is_empty(),
                        on_undo: move |()| {
                            let action = edit_history.write().pop();
                            let result = match &action {
                                Some(GraphEditorAction::Topology) => draft
                                    .write()
                                    .as_mut()
                                    .map_or_else(
                                        || Err("Graph draft is unavailable".to_owned()),
                                        |draft| draft.undo().map_err(|error| error.to_string()),
                                    ),
                                Some(GraphEditorAction::Layout(previous)) => {
                                    node_positions.set(previous.clone());
                                    Ok(())
                                }
                                Some(GraphEditorAction::TopologyAndLayout(previous)) => {
                                    node_positions.set(previous.clone());
                                    draft.write().as_mut().map_or_else(
                                        || Err("Graph draft is unavailable".to_owned()),
                                        |draft| draft.undo().map_err(|error| error.to_string()),
                                    )
                                }
                                None => Err("There is no Graph edit to undo.".to_owned()),
                            };
                            match result {
                                Ok(()) => {
                                    on_layout_save.call(graph_canvas_layout_from_positions(
                                        &undo_layout_graph_id,
                                        &node_positions.read(),
                                    ));
                                    let next_selection = draft
                                        .read()
                                        .as_ref()
                                        .and_then(|draft| draft.graph().entries.first())
                                        .map_or_else(String::new, |entry| entry.node_id.clone());
                                    selected_node_id.set(next_selection);
                                    editor_notice.set("Last Graph edit undone.".to_owned());
                                }
                                Err(error) => {
                                    if let Some(action) = action {
                                        edit_history.write().push(action);
                                    }
                                    editor_notice.set(error);
                                }
                            }
                        },
                        on_reset: move |()| {
                            let result = draft
                                .write()
                                .as_mut()
                                .map_or_else(
                                    || Err("Graph draft is unavailable".to_owned()),
                                    |draft| draft.reset().map_err(|error| error.to_string()),
                                );
                            match result {
                                Ok(()) => {
                                    node_positions.write().clear();
                                    on_layout_save.call(graph_canvas_layout_from_positions(
                                        &reset_layout_graph_id,
                                        &node_positions.read(),
                                    ));
                                    edit_history.write().clear();
                                    let next_selection = draft
                                        .read()
                                        .as_ref()
                                        .and_then(|draft| draft.graph().entries.first())
                                        .map_or_else(String::new, |entry| entry.node_id.clone());
                                    selected_node_id.set(next_selection);
                                    editor_notice.set("Graph draft reset to the live revision.".to_owned());
                                }
                                Err(error) => editor_notice.set(error),
                            }
                        },
                        on_auto_arrange: move |()| {
                            let previous = node_positions.read().clone();
                            if !previous.is_empty() {
                                edit_history.write().push(GraphEditorAction::Layout(previous));
                                node_positions.write().clear();
                                on_layout_save.call(graph_canvas_layout_from_positions(
                                    &arrange_layout_graph_id,
                                    &node_positions.read(),
                                ));
                                editor_notice.set("Nodes returned to automatic layout.".to_owned());
                            }
                        },
                        on_move_node: move |(node_id, point): (String, CanvasPoint)| {
                            let previous = node_positions.read().clone();
                            if previous.get(&node_id).copied() != Some(point) {
                                edit_history.write().push(GraphEditorAction::Layout(previous));
                                node_positions.write().insert(node_id.clone(), point);
                                on_layout_save.call(graph_canvas_layout_from_positions(
                                    &move_layout_graph_id,
                                    &node_positions.read(),
                                ));
                                selected_node_id.set(node_id);
                                editor_notice.set("Node position updated.".to_owned());
                            }
                        },
                        on_spawn_node: move |(kind_value, point, pending_branch): (String, CanvasPoint, Option<PendingControlBranchSpawn>)| {
                            let node_id = draft
                                .read()
                                .as_ref()
                                .map_or_else(
                                    || "new-node".to_owned(),
                                    |draft| next_canvas_node_id(draft.graph(), &kind_value),
                                );
                            let kind = control_node_kind_from_editor(
                                &kind_value,
                                &palette_agent_profile_id,
                            );
                            let previous_positions = node_positions.read().clone();
                            let branch_for_connection = pending_branch.clone();
                            let result = kind.map_or_else(
                                || Err("Choose an Agent profile below before dropping an Agent Loop.".to_owned()),
                                |kind| draft
                                    .write()
                                    .as_mut()
                                    .map_or_else(
                                        || Err("Graph draft is unavailable".to_owned()),
                                        |draft| {
                                            draft
                                                .add_node(node_id.clone(), kind)
                                                .map_err(|error| error.to_string())?;
                                            if let Some(pending) = &branch_for_connection {
                                                let route_id = next_graph_route_id(
                                                    &pending.source_node_id,
                                                    &node_id,
                                                    pending.signal,
                                                );
                                                if let Err(error) = draft.connect(
                                                    route_id,
                                                    pending.source_node_id.clone(),
                                                    node_id.clone(),
                                                    pending.signal,
                                                ) {
                                                    let _ = draft.undo();
                                                    return Err(error.to_string());
                                                }
                                            }
                                            Ok(())
                                        },
                                    ),
                            );
                            match result {
                                Ok(()) => {
                                    edit_history.write().push(
                                        GraphEditorAction::TopologyAndLayout(previous_positions),
                                    );
                                    if pending_branch.is_some() {
                                        edit_history.write().push(GraphEditorAction::Topology);
                                    }
                                    node_positions.write().insert(node_id.clone(), point);
                                    on_layout_save.call(graph_canvas_layout_from_positions(
                                        &spawn_layout_graph_id,
                                        &node_positions.read(),
                                    ));
                                    selected_node_id.set(node_id);
                                    editor_notice.set(if pending_branch.is_some() {
                                        "A new node was created and connected to the branch.".to_owned()
                                    } else {
                                        "A new node was created at the drop position.".to_owned()
                                    });
                                }
                                Err(error) => editor_notice.set(error),
                            }
                        },
                        on_select: move |selected: String| {
                            selected_node_id.set(selected);
                            replacement_agent_profile_id.set(String::new());
                            editor_notice.set(String::new());
                        },
                        on_remove_route: move |route_id: String| {
                            let result = draft
                                .write()
                                .as_mut()
                                .map_or_else(
                                    || Err("Graph draft is unavailable".to_owned()),
                                    |draft| draft.remove_route(&route_id).map_err(|error| error.to_string()),
                            );
                            match result {
                                Ok(()) => {
                                    edit_history.write().push(GraphEditorAction::Topology);
                                    editor_notice.set("Connection removed from the draft.".to_owned());
                                }
                                Err(error) => editor_notice.set(error),
                            }
                        },
                        on_connect: move |(source_node_id, destination_node_id, signal): (String, String, ControlSignal)| {
                            let result = connect_graph_draft(
                                &mut draft,
                                source_node_id.clone(),
                                destination_node_id,
                                signal,
                            );
                            match result {
                                Ok(()) => {
                                    edit_history.write().push(GraphEditorAction::Topology);
                                    selected_node_id.set(source_node_id);
                                    editor_notice.set("Nodes connected through their sockets.".to_owned());
                                }
                                Err(error) => editor_notice.set(error),
                            }
                        }
                    }
                    div { class: "graph-builder-tools",
                        section { class: "graph-builder-tool",
                            h4 { "Add node" }
                            label {
                                span { "Node name" }
                                input {
                                    aria_label: "New Graph node name",
                                    value: "{new_node_id_value}",
                                    maxlength: 128,
                                    placeholder: "security-review",
                                    oninput: move |event| new_node_id.set(event.value()),
                                }
                            }
                            label {
                                span { "Node type" }
                                select {
                                    aria_label: "New Graph node type",
                                    value: "{new_node_kind_value}",
                                    onchange: move |event| new_node_kind.set(event.value()),
                                    option { value: "agent_loop", "Agent Loop" }
                                    option { value: "gate", "Gate" }
                                    option { value: "approval", "Approval" }
                                    option { value: "audit", "Audit" }
                                    option { value: "terminal", "Terminal" }
                                }
                            }
                            if new_node_kind_value == "agent_loop" {
                                label {
                                    span { "Agent profile" }
                                    select {
                                        aria_label: "New Graph node Agent profile",
                                        value: "{new_node_profile_value}",
                                        onchange: move |event| new_node_profile_id.set(event.value()),
                                        option { value: "", "Select profile" }
                                        for profile in &agent_profiles {
                                            option { value: "{profile.id}", "{profile.role} · {profile.id}" }
                                        }
                                    }
                                }
                            }
                            button {
                                disabled: !can_add_node,
                                onclick: move |_| {
                                    let node_id = new_node_id_value.trim().to_owned();
                                    let kind = control_node_kind_from_editor(
                                        &new_node_kind_value,
                                        &new_node_profile_value,
                                    );
                                    let result = kind.map_or_else(
                                        || Err("Choose a node type and Agent profile.".to_owned()),
                                        |kind| {
                                            draft
                                                .write()
                                                .as_mut()
                                                .map_or_else(
                                                    || Err("Graph draft is unavailable".to_owned()),
                                                    |draft| draft
                                                        .add_node(node_id.clone(), kind)
                                                        .map_err(|error| error.to_string()),
                                                )
                                        },
                                    );
                                    match result {
                                        Ok(()) => {
                                            edit_history.write().push(GraphEditorAction::Topology);
                                            selected_node_id.set(node_id);
                                            new_node_id.set(String::new());
                                            new_node_profile_id.set(String::new());
                                            editor_notice.set("Node added. Connect it to make the draft ready.".to_owned());
                                        }
                                        Err(error) => editor_notice.set(error),
                                    }
                                },
                                "Add node"
                            }
                        }
                        section { class: "graph-builder-tool",
                            h4 { "Connect selected node" }
                            p { class: "graph-builder-selection", "From: {node_id_value}" }
                            label {
                                span { "To" }
                                select {
                                    aria_label: "Graph connection destination",
                                    value: "{connection_target_value}",
                                    onchange: move |event| connection_target_id.set(event.value()),
                                    option { value: "", "Select destination" }
                                    if let Some(graph) = &draft_graph {
                                        for node in &graph.nodes {
                                            option {
                                                value: "{node.id}",
                                                disabled: node.id == node_id_value,
                                                "{node.id}"
                                            }
                                        }
                                    }
                                }
                            }
                            label {
                                span { "When" }
                                select {
                                    aria_label: "Graph connection signal",
                                    value: "{control_signal_value(selected_connection_signal)}",
                                    onchange: move |event| connection_signal.set(event.value()),
                                    for signal in allowed_connection_signals {
                                        option {
                                            value: "{control_signal_value(*signal)}",
                                            "{control_signal_label(*signal)}"
                                        }
                                    }
                                }
                            }
                            button {
                                disabled: node_id_value.is_empty()
                                    || connection_target_value.is_empty()
                                    || allowed_connection_signals.is_empty(),
                                onclick: move |_| {
                                    let result = connect_graph_draft(
                                        &mut draft,
                                        connect_source_node_id.clone(),
                                        connection_target_value.clone(),
                                        selected_connection_signal,
                                    );
                                    match result {
                                        Ok(()) => {
                                            edit_history.write().push(GraphEditorAction::Topology);
                                            connection_target_id.set(String::new());
                                            editor_notice.set("Nodes connected in the draft.".to_owned());
                                        }
                                        Err(error) => editor_notice.set(error),
                                    }
                                },
                                "Connect"
                            }
                            if allowed_connection_signals.is_empty() {
                                small { "Terminal nodes finish a route and cannot start another connection." }
                            }
                        }
                        section { class: "graph-builder-tool",
                            h4 { "Selected node" }
                            if let Some(node) = &selected_node {
                                strong { "{node.id}" }
                                small { "{control_node_kind_label(&node.kind)}" }
                                if let Some(current_profile) = &current_agent_profile_id {
                                    label {
                                        span { "Change Agent profile" }
                                        select {
                                            aria_label: "Replacement Agent profile",
                                            value: "{replacement_value}",
                                            disabled: topology_changed,
                                            onchange: move |event| replacement_agent_profile_id.set(event.value()),
                                            option { value: "", "Keep {current_profile}" }
                                            for profile in &agent_profiles {
                                                option {
                                                    value: "{profile.id}",
                                                    disabled: profile.id == *current_profile,
                                                    "{profile.role} · {profile.id}"
                                                }
                                            }
                                        }
                                    }
                                }
                                button {
                                    class: "graph-node-remove",
                                    disabled: selected_is_entry,
                                    title: if selected_is_entry { "Entry nodes stay fixed" } else { "Remove this node and its connections" },
                                    onclick: move |_| {
                                        let result = draft
                                            .write()
                                            .as_mut()
                                            .map_or_else(
                                                || Err("Graph draft is unavailable".to_owned()),
                                                |draft| draft.remove_node(&removable_node_id).map_err(|error| error.to_string()),
                                        );
                                        match result {
                                            Ok(()) => {
                                                edit_history.write().push(GraphEditorAction::Topology);
                                                node_positions.write().remove(&removable_node_id);
                                                on_layout_save.call(graph_canvas_layout_from_positions(
                                                    &remove_layout_graph_id,
                                                    &node_positions.read(),
                                                ));
                                                selected_node_id.set(String::new());
                                                editor_notice.set("Node and its connections removed from the draft.".to_owned());
                                            }
                                            Err(error) => editor_notice.set(error),
                                        }
                                    },
                                    if selected_is_entry { "Entry node (fixed)" } else { "Remove node" }
                                }
                            } else {
                                small { "Choose a node on the map." }
                            }
                        }
                    }
                    if !editor_notice_value.is_empty() {
                        p { class: "graph-editor-notice", "{editor_notice_value}" }
                    }
                    if topology_changed && !topology_is_valid {
                        p { class: "field-warning", "Connect every node to a path from an entry before requesting approval." }
                    }
                    div { class: "graph-rewrite-form",
                        label {
                            span { "Why this change" }
                            textarea {
                                aria_label: "Graph change rationale",
                                value: "{rationale_value}",
                                maxlength: 1024,
                                placeholder: "What this route or stage improves",
                                oninput: move |event| rationale.set(event.value()),
                            }
                        }
                        label {
                            span { "Evidence reference" }
                            input {
                                aria_label: "Graph change evidence reference",
                                value: "{evidence_ref_value}",
                                maxlength: 512,
                                placeholder: "checkpoint:cp-42",
                                oninput: move |event| evidence_ref.set(event.value()),
                            }
                        }
                        button {
                            class: "graph-rewrite-create",
                            disabled: !can_propose,
                            onclick: move |_| {
                                let (proposal_id, candidate_revision_id) = next_graph_rewrite_ids();
                                let operation = if topology_changed {
                                    draft
                                        .read()
                                        .as_ref()
                                        .and_then(|draft| draft.candidate(&candidate_revision_id).ok())
                                        .map(|(_, operation)| operation)
                                } else {
                                    current_agent_profile_id.clone().map(|previous_agent_profile_id| {
                                        GraphRewriteOperation::ReplaceAgentProfile {
                                            node_id: proposal_node_id.clone(),
                                            previous_agent_profile_id,
                                            replacement_agent_profile_id: replacement_value.clone(),
                                        }
                                    })
                                };
                                if let Some(operation) = operation {
                                    on_propose.call(GraphRewriteProposalRequest {
                                        proposal_id,
                                        project_id: project_id.clone(),
                                        expected_source_binding: binding.clone(),
                                        candidate_revision_id,
                                        operation,
                                        rationale: rationale_value.trim().to_owned(),
                                        evidence_refs: vec![evidence_ref_value.trim().to_owned()],
                                    });
                                } else {
                                    editor_notice.set("Finish connecting the graph before requesting approval.".to_owned());
                                }
                            },
                            "Request approval"
                        }
                    }
                } else {
                    small { "This Graph revision could not be opened in the builder." }
                }
            } else {
                small { "Select a Control graph before proposing a change." }
            }
            if proposals.is_empty() {
                small { "No Graph change proposals recorded for this project." }
            } else {
                div { class: "graph-rewrite-list",
                    for proposal in proposals {
                        GraphRewriteProposalCard {
                            key: "{proposal.proposal_id}",
                            proposal,
                            on_decide,
                        }
                    }
                }
            }
        }
    }
}

fn graph_canvas_drop_point(x: f64, y: f64, zoom: f64) -> CanvasPoint {
    CanvasPoint {
        x: (x / zoom - 82.0).max(8.0),
        y: (y / zoom - 42.0).max(8.0),
    }
}

fn graph_drag_preview_point(
    start_position: CanvasPoint,
    pointer_origin: CanvasPoint,
    pointer_current: CanvasPoint,
    zoom: f64,
) -> CanvasPoint {
    let safe_zoom = zoom.max(f64::EPSILON);
    CanvasPoint {
        x: (start_position.x + (pointer_current.x - pointer_origin.x) / safe_zoom).max(8.0),
        y: (start_position.y + (pointer_current.y - pointer_origin.y) / safe_zoom).max(8.0),
    }
}

fn pending_control_branch_spawn(
    source_node_id: &str,
    signal: Option<ControlSignal>,
    point: CanvasPoint,
) -> Option<PendingControlBranchSpawn> {
    (!source_node_id.is_empty()).then_some(PendingControlBranchSpawn {
        source_node_id: source_node_id.to_owned(),
        signal: signal?,
        point,
    })
}

fn install_graph_canvas_pan(viewport_id: &str) {
    let script = format!(
        r"
        (() => {{
          const viewport = document.getElementById({viewport_id:?});
          if (!viewport || viewport.dataset.panReady === 'true') return;
          viewport.dataset.panReady = 'true';
          let active = false;
          let originX = 0;
          let originY = 0;
          let scrollX = 0;
          let scrollY = 0;
          viewport.addEventListener('pointerdown', (event) => {{
            if (event.button !== 0 && event.button !== 1) return;
            if (event.target.closest('button, .graph-node, .graph-spawn-menu')) return;
            active = true;
            originX = event.clientX;
            originY = event.clientY;
            scrollX = viewport.scrollLeft;
            scrollY = viewport.scrollTop;
            viewport.classList.add('panning');
            viewport.setPointerCapture(event.pointerId);
          }});
          viewport.addEventListener('pointermove', (event) => {{
            if (!active) return;
            viewport.scrollLeft = scrollX - (event.clientX - originX);
            viewport.scrollTop = scrollY - (event.clientY - originY);
          }});
          const finish = (event) => {{
            if (!active) return;
            active = false;
            viewport.classList.remove('panning');
            if (viewport.hasPointerCapture(event.pointerId)) {{
              viewport.releasePointerCapture(event.pointerId);
            }}
          }};
          viewport.addEventListener('pointerup', finish);
          viewport.addEventListener('pointercancel', finish);
        }})();
        "
    );
    let _ = document::eval(&script);
}

#[component]
fn GraphCanvas(
    graph: ControlGraphRevision,
    node_positions: HashMap<String, CanvasPoint>,
    selected_node_id: String,
    connection_signal: ControlSignal,
    can_undo: bool,
    can_reset: bool,
    can_auto_arrange: bool,
    on_undo: EventHandler<()>,
    on_reset: EventHandler<()>,
    on_auto_arrange: EventHandler<()>,
    on_move_node: EventHandler<(String, CanvasPoint)>,
    on_spawn_node: EventHandler<(String, CanvasPoint, Option<PendingControlBranchSpawn>)>,
    on_select: EventHandler<String>,
    on_remove_route: EventHandler<String>,
    on_connect: EventHandler<(String, String, ControlSignal)>,
) -> Element {
    let mut dragged_node_id = use_signal(String::new);
    let mut dragged_route_source_id = use_signal(String::new);
    let mut dragged_route_signal = use_signal(|| None::<ControlSignal>);
    let mut dragged_palette_kind = use_signal(String::new);
    let mut drop_target_node_id = use_signal(String::new);
    let mut spawn_menu_position = use_signal(|| None::<CanvasPoint>);
    let mut pending_branch_spawn = use_signal(|| None::<PendingControlBranchSpawn>);
    let mut zoom_percent = use_signal(|| 100_u32);
    let mut drag_origin = use_signal(|| None::<CanvasPoint>);
    let mut drag_start_position = use_signal(|| None::<CanvasPoint>);
    let mut pointer_drag_active = use_signal(|| false);
    let mut drag_preview = use_signal(|| None::<(String, CanvasPoint)>);
    let drop_target_value = drop_target_node_id.read().clone();
    let dragged_node_value = dragged_node_id.read().clone();
    let pointer_drag_value = *pointer_drag_active.read();
    let spawn_menu_value = *spawn_menu_position.read();
    let pending_branch_value = pending_branch_spawn.read().clone();
    let spawn_menu_style = spawn_menu_value.map_or_else(String::new, |point| {
        format!("left: {:.1}px; top: {:.1}px;", point.x, point.y)
    });
    let layout = ControlGraphLayout::new(&graph);
    let zoom = f64::from(*zoom_percent.read()) / 100.0;
    let zoom_label = format!("{}%", *zoom_percent.read());
    let pan_viewport_id = format!("graph-pan-{}", graph.graph_id);
    let pan_viewport_effect_id = pan_viewport_id.clone();
    use_effect(move || install_graph_canvas_pan(&pan_viewport_effect_id));
    let mut rendered_positions = node_positions.clone();
    if let Some((node_id, point)) = drag_preview.read().clone() {
        rendered_positions.insert(node_id, point);
    }
    let palette_spawn_point = layout.next_open_canvas_point(&rendered_positions);
    let (stage_width, stage_height) = layout.freeform_stage_dimensions(&rendered_positions);
    let stage_style = format!("width: {stage_width}px; min-height: {stage_height}px;");
    let scaled_stage_style = format!(
        "width: {:.1}px; height: {:.1}px;",
        stage_width * zoom,
        stage_height * zoom,
    );
    let zoomed_stage_style =
        format!("{stage_style} transform: scale({zoom:.2}); transform-origin: top left;");
    let view_box = format!("0 0 {stage_width} {stage_height}");
    rsx! {
        section { class: "graph-canvas", aria_label: "Control Graph builder canvas",
            header { class: "graph-canvas-heading",
                div {
                    strong { "Draft map" }
                    small { "Drag nodes freely · drop a labeled branch pin on a node to connect, or on empty canvas to create its destination" }
                }
                div { class: "graph-canvas-heading-tools",
                    div { class: "graph-canvas-actions",
                        button {
                            disabled: !can_undo,
                            title: "Undo the last draft edit",
                            onclick: move |_| on_undo.call(()),
                            "Undo"
                        }
                        button {
                            disabled: !can_reset,
                            title: "Reset every draft edit",
                            onclick: move |_| on_reset.call(()),
                            "Reset"
                        }
                        button {
                            disabled: !can_auto_arrange,
                            title: "Return moved nodes to automatic layout",
                            onclick: move |_| on_auto_arrange.call(()),
                            "Auto arrange"
                        }
                        span { class: "graph-canvas-action-divider" }
                        button {
                            disabled: *zoom_percent.read() <= 60,
                            title: "Zoom out",
                            onclick: move |_| {
                                let next = zoom_percent.read().saturating_sub(20).max(60);
                                zoom_percent.set(next);
                            },
                            "−"
                        }
                        output { class: "graph-zoom-value", aria_label: "Canvas zoom", "{zoom_label}" }
                        button {
                            disabled: *zoom_percent.read() >= 160,
                            title: "Zoom in",
                            onclick: move |_| {
                                let next = zoom_percent.read().saturating_add(20).min(160);
                                zoom_percent.set(next);
                            },
                            "+"
                        }
                        button {
                            disabled: *zoom_percent.read() == 100,
                            title: "Reset zoom to 100%",
                            onclick: move |_| zoom_percent.set(100),
                            "100"
                        }
                    }
                    div { class: "graph-canvas-stats",
                        span { "Freeform" }
                        span { "{graph.nodes.len()} nodes · {graph.routes.len()} connections" }
                    }
                }
            }
            div { class: "graph-node-palette", aria_label: "Node palette",
                span { "Create node" }
                for (kind_value, label, hint) in [
                    ("agent_loop", "Agent Loop", "Runs one reusable agent approach"),
                    ("gate", "Gate", "Chooses the next route"),
                    ("audit", "Audit", "Records a verification step"),
                    ("approval", "Approval", "Waits for a human decision"),
                    ("terminal", "Terminal", "Finishes this route"),
                ] {
                    button {
                        class: "graph-palette-node",
                        title: "{hint}",
                        onpointerdown: move |_| {
                            dragged_palette_kind.set(kind_value.to_owned());
                        },
                        onclick: move |event| {
                            event.stop_propagation();
                            dragged_palette_kind.set(String::new());
                            on_spawn_node.call((kind_value.to_owned(), palette_spawn_point, None));
                        },
                        "{label}"
                    }
                }
                small { "Drag into empty space, click for the next open slot, or double-click. Drag blank space to pan." }
            }
            div { class: "graph-layout-scroll", id: "{pan_viewport_id}",
                div {
                    class: "graph-stage-scale",
                    style: "{scaled_stage_style}",
                    div {
                    class: "graph-stage graph-freeform-stage",
                    style: "{zoomed_stage_style}",
                    onpointermove: move |event| {
                        let node_id = dragged_node_id.read().clone();
                        if node_id.is_empty() {
                            return;
                        }
                        let current = event.client_coordinates();
                        let current = CanvasPoint { x: current.x, y: current.y };
                        let origin = *drag_origin.read();
                        let exceeded = origin
                            .is_some_and(|origin| pointer_drag_exceeded(origin, current));
                        if exceeded || *pointer_drag_active.read() {
                            pointer_drag_active.set(true);
                            if let (Some(origin), Some(start_position)) =
                                (origin, *drag_start_position.read())
                            {
                                drag_preview.set(Some((
                                    node_id,
                                    graph_drag_preview_point(
                                        start_position,
                                        origin,
                                        current,
                                        zoom,
                                    ),
                                )));
                            }
                        }
                    },
                    onpointerup: move |event| {
                        let coordinates = event.element_coordinates();
                        let point = graph_canvas_drop_point(coordinates.x, coordinates.y, zoom);
                        let kind_value = dragged_palette_kind.read().clone();
                        let node_id = dragged_node_id.read().clone();
                        let source_node_id = dragged_route_source_id.read().clone();
                        if !kind_value.is_empty() {
                            on_spawn_node.call((kind_value, point, None));
                        } else if !node_id.is_empty() && *pointer_drag_active.read() {
                            let final_point = drag_preview
                                .read()
                                .as_ref()
                                .filter(|(preview_node_id, _)| preview_node_id == &node_id)
                                .map(|(_, preview_point)| *preview_point)
                                .or(*drag_start_position.read());
                            if let Some(final_point) = final_point {
                                on_move_node.call((node_id, final_point));
                            }
                        } else if let Some(pending) = pending_control_branch_spawn(
                            &source_node_id,
                            *dragged_route_signal.read(),
                            point,
                        ) {
                            pending_branch_spawn.set(Some(pending));
                            spawn_menu_position.set(Some(point));
                        }
                        dragged_palette_kind.set(String::new());
                        dragged_node_id.set(String::new());
                        dragged_route_source_id.set(String::new());
                        dragged_route_signal.set(None);
                        drop_target_node_id.set(String::new());
                        drag_origin.set(None);
                        drag_start_position.set(None);
                        pointer_drag_active.set(false);
                        drag_preview.set(None);
                    },
                    onpointercancel: move |_| {
                        dragged_node_id.set(String::new());
                        dragged_route_source_id.set(String::new());
                        dragged_route_signal.set(None);
                        dragged_palette_kind.set(String::new());
                        drop_target_node_id.set(String::new());
                        drag_origin.set(None);
                        drag_start_position.set(None);
                        pointer_drag_active.set(false);
                        drag_preview.set(None);
                    },
                    ondoubleclick: move |event| {
                        event.stop_propagation();
                        let coordinates = event.element_coordinates();
                        pending_branch_spawn.set(None);
                        spawn_menu_position.set(Some(graph_canvas_drop_point(
                            coordinates.x,
                            coordinates.y,
                            zoom,
                        )));
                    },
                    svg {
                        class: "graph-route-layer",
                        view_box: "{view_box}",
                        width: "{stage_width}",
                        height: "{stage_height}",
                        defs {
                            marker {
                                id: "graph-route-arrow",
                                marker_width: "7",
                                marker_height: "7",
                                ref_x: "6",
                                ref_y: "3",
                                orient: "auto",
                                marker_units: "strokeWidth",
                                path { d: "M 0 0 L 6 3 L 0 6 z" }
                            }
                        }
                        for route in &graph.routes {
                            if let Some(path_data) = layout.freeform_branch_route_path(
                                &route.source_node_id,
                                &route.destination_node_id,
                                &rendered_positions,
                                control_route_source_port_y(
                                    &graph,
                                    &route.source_node_id,
                                    route.signal,
                                    connection_signal,
                                ),
                                GRAPH_INPUT_PORT_Y,
                            ) {
                                path {
                                    key: "path-{route.id}",
                                    class: format!(
                                        "graph-route-path signal-{}{}",
                                        control_signal_value(route.signal),
                                        if route.source_node_id == selected_node_id
                                            || route.destination_node_id == selected_node_id
                                        {
                                            " active"
                                        } else {
                                            ""
                                        },
                                    ),
                                    d: "{path_data}",
                                    marker_end: "url(#graph-route-arrow)",
                                }
                            }
                        }
                    }
                    div { class: "graph-freeform-nodes",
                        for node in &graph.nodes {
                            {
                                let can_start_route = !matches!(node.kind, ControlNodeKind::Terminal);
                                let point = rendered_positions
                                    .get(&node.id)
                                    .copied()
                                    .or_else(|| layout.canvas_point(&node.id))
                                    .unwrap_or_default();
                                let outgoing_signals = graph
                                    .routes
                                    .iter()
                                    .filter(|route| route.source_node_id == node.id)
                                    .map(|route| route.signal)
                                    .collect::<Vec<_>>();
                                let branch_signals = branch_signal_options(
                                    control_signals_for_node(Some(&node.kind)),
                                    primary_control_branch_signals(&node.kind),
                                    &outgoing_signals,
                                    Some(connection_signal),
                                );
                                let node_style =
                                    graph_freeform_node_style(point, branch_signals.len());
                                rsx! {
                                    button {
                                        key: "{node.id}",
                                        class: match (
                                            node.id == dragged_node_value && pointer_drag_value,
                                            node.id == drop_target_value,
                                            node.id == selected_node_id,
                                            layout.is_reachable(&node.id),
                                            can_start_route,
                                        ) {
                                            (true, _, _, _, _) => "graph-node graph-freeform-node dragging",
                                            (_, true, _, _, _) => "graph-node graph-freeform-node drop-target",
                                            (_, _, true, _, false) => "graph-node graph-freeform-node selected terminal",
                                            (_, _, true, _, true) => "graph-node graph-freeform-node selected",
                                            (_, _, _, false, _) => "graph-node graph-freeform-node unreachable",
                                            (_, _, _, _, false) => "graph-node graph-freeform-node terminal",
                                            _ => "graph-node graph-freeform-node",
                                        },
                                        style: "{node_style}",
                                        aria_pressed: node.id == selected_node_id,
                                        title: "Drag the card to move it. Use the right socket to create a connection.",
                                        onclick: {
                                            let node_id = node.id.clone();
                                            move |_| on_select.call(node_id.clone())
                                        },
                                        ondoubleclick: move |event| event.stop_propagation(),
                                        onpointerdown: {
                                            let node_id = node.id.clone();
                                            let start_position = point;
                                            move |event| {
                                                if dragged_route_source_id.read().is_empty() {
                                                    let origin = event.client_coordinates();
                                                    dragged_node_id.set(node_id.clone());
                                                    dragged_route_signal.set(None);
                                                    drag_origin.set(Some(CanvasPoint {
                                                        x: origin.x,
                                                        y: origin.y,
                                                    }));
                                                    drag_start_position.set(Some(start_position));
                                                    pointer_drag_active.set(false);
                                                    drag_preview.set(None);
                                                    on_select.call(node_id.clone());
                                                }
                                            }
                                        },
                                        onpointerenter: {
                                            let destination_node_id = node.id.clone();
                                            move |_| {
                                                let source_node_id = dragged_route_source_id.read();
                                                if !source_node_id.is_empty()
                                                    && source_node_id.as_str() != destination_node_id
                                                {
                                                    drop_target_node_id.set(destination_node_id.clone());
                                                }
                                            }
                                        },
                                        onpointerup: {
                                            let destination_node_id = node.id.clone();
                                            move |event| {
                                                let source_node_id = dragged_route_source_id.read().clone();
                                                let signal = *dragged_route_signal.read();
                                                if !source_node_id.is_empty() {
                                                    event.stop_propagation();
                                                    if source_node_id != destination_node_id
                                                        && let Some(signal) = signal
                                                    {
                                                        on_connect.call((source_node_id, destination_node_id.clone(), signal));
                                                    }
                                                    dragged_route_source_id.set(String::new());
                                                    dragged_route_signal.set(None);
                                                    drop_target_node_id.set(String::new());
                                                }
                                            }
                                        },
                                        span {
                                            class: "graph-node-socket graph-node-socket-input",
                                            title: "Input socket",
                                            aria_hidden: "true",
                                        }
                                        if graph.entries.iter().any(|entry| entry.node_id == node.id) {
                                            span { class: "graph-node-entry", "Entry" }
                                        } else if !layout.is_reachable(&node.id) {
                                            span { class: "graph-node-entry graph-node-unconnected", "Connect me" }
                                        }
                                        strong { "{node.id}" }
                                        small { "{control_node_kind_label(&node.kind)}" }
                                        if can_start_route {
                                            div { class: "graph-node-branches", aria_label: "Branches from {node.id}",
                                                for (branch_index, signal) in branch_signals.into_iter().enumerate() {
                                                    {
                                                        let branch_style = graph_branch_pin_style(branch_index);
                                                        let destination = graph.routes.iter().find_map(|route| {
                                                            (route.source_node_id == node.id && route.signal == signal)
                                                                .then_some(route.destination_node_id.clone())
                                                        });
                                                        let branch_class = if destination.is_some() {
                                                            "graph-node-branch-pin connected"
                                                        } else {
                                                            "graph-node-branch-pin"
                                                        };
                                                        let can_connect = destination.is_none();
                                                        let signal_label = control_signal_label(signal);
                                                        let branch_title = destination.as_ref().map_or_else(
                                                            || format!("Drag the {signal_label} branch to another node"),
                                                            |destination| format!("{signal_label} connects to {destination}. Remove it before reconnecting."),
                                                        );
                                                        rsx! {
                                                            span {
                                                                class: "{branch_class}",
                                                                style: "{branch_style}",
                                                                title: "{branch_title}",
                                                                aria_label: "Connect {signal_label} from {node.id}",
                                                                onpointerdown: {
                                                                    let node_id = node.id.clone();
                                                                    move |event| {
                                                                        event.stop_propagation();
                                                                        if !can_connect {
                                                                            return;
                                                                        }
                                                                        dragged_node_id.set(String::new());
                                                                        drag_origin.set(None);
                                                                        drag_start_position.set(None);
                                                                        pointer_drag_active.set(false);
                                                                        drag_preview.set(None);
                                                                        dragged_route_source_id.set(node_id.clone());
                                                                        dragged_route_signal.set(Some(signal));
                                                                    }
                                                                },
                                                                span { class: "graph-node-branch-label", "{signal_label}" }
                                                                span { class: "graph-node-socket graph-node-socket-output", aria_hidden: "true" }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if let Some(point) = spawn_menu_value {
                        div {
                            class: "graph-spawn-menu",
                            style: "{spawn_menu_style}",
                            strong {
                                if pending_branch_value.is_some() {
                                    "Create branch destination"
                                } else {
                                    "Create node"
                                }
                            }
                            if let Some(pending) = &pending_branch_value {
                                small { "{control_signal_label(pending.signal)} from {pending.source_node_id}" }
                            }
                            for (kind_value, label) in [
                                ("agent_loop", "Agent Loop"),
                                ("gate", "Gate"),
                                ("audit", "Audit"),
                                ("approval", "Approval"),
                                ("terminal", "Terminal"),
                            ] {
                                {
                                    let pending_for_spawn = pending_branch_value.clone();
                                    rsx! {
                                        button {
                                            onclick: move |event| {
                                                event.stop_propagation();
                                                on_spawn_node.call((
                                                    kind_value.to_owned(),
                                                    point,
                                                    pending_for_spawn.clone(),
                                                ));
                                                spawn_menu_position.set(None);
                                                pending_branch_spawn.set(None);
                                            },
                                            "{label}"
                                        }
                                    }
                                }
                            }
                            button {
                                class: "graph-spawn-menu-cancel",
                                onclick: move |event| {
                                    event.stop_propagation();
                                    spawn_menu_position.set(None);
                                    pending_branch_spawn.set(None);
                                },
                                "Cancel"
                            }
                        }
                    }
                    }
                }
            }
            div { class: "graph-route-map",
                if graph.routes.is_empty() {
                    small { "No connections yet." }
                }
                for route in &graph.routes {
                    div { class: "graph-route-edge", key: "{route.id}",
                        button {
                            class: if route.source_node_id == selected_node_id { "graph-route-node active" } else { "graph-route-node" },
                            onclick: {
                                let source = route.source_node_id.clone();
                                move |_| on_select.call(source.clone())
                            },
                            "{route.source_node_id}"
                        }
                        span { class: "graph-route-line",
                            small { "{control_signal_label(route.signal)}" }
                            span { "→" }
                        }
                        button {
                            class: if route.destination_node_id == selected_node_id { "graph-route-node active" } else { "graph-route-node" },
                            onclick: {
                                let destination = route.destination_node_id.clone();
                                move |_| on_select.call(destination.clone())
                            },
                            "{route.destination_node_id}"
                        }
                        button {
                            class: "graph-route-remove",
                            aria_label: "Remove connection {route.id}",
                            title: "Remove connection from draft",
                            onclick: {
                                let route_id = route.id.clone();
                                move |_| on_remove_route.call(route_id.clone())
                            },
                            "×"
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn GraphRewriteProposalCard(
    proposal: GraphRewriteProposal,
    on_decide: EventHandler<GraphRewriteDecisionRequest>,
) -> Element {
    let change_summary = match &proposal.operation {
        GraphRewriteOperation::ReplaceAgentProfile {
            node_id,
            previous_agent_profile_id,
            replacement_agent_profile_id,
        } => format!("{node_id}: {previous_agent_profile_id} → {replacement_agent_profile_id}"),
        GraphRewriteOperation::EditTopology {
            added_nodes,
            removed_node_ids,
            added_routes,
            removed_route_ids,
        } => format!(
            "Topology: +{} node(s), −{} node(s), +{} route(s), −{} route(s)",
            added_nodes.len(),
            removed_node_ids.len(),
            added_routes.len(),
            removed_route_ids.len(),
        ),
    };
    let is_pending = proposal.status == GraphRewriteProposalStatus::Pending;
    rsx! {
        article { class: "graph-rewrite-proposal",
            div { class: "graph-rewrite-proposal-heading",
                code { "{proposal.candidate_graph.revision_id}" }
                strong { "{graph_rewrite_status_label(proposal.status)}" }
            }
            p { "{change_summary}" }
            p { "{proposal.rationale}" }
            div { class: "route-decision-evidence",
                span { "Evidence" }
                for evidence_ref in &proposal.evidence_refs {
                    code { "{evidence_ref}" }
                }
            }
            if is_pending {
                div { class: "graph-rewrite-actions",
                    button {
                        class: "approval-action",
                        onclick: {
                            let request = GraphRewriteDecisionRequest {
                                proposal_id: proposal.proposal_id.clone(),
                                project_id: proposal.project_id.clone(),
                                decision: GraphRewriteDecision::Approve,
                            };
                            move |_| on_decide.call(request.clone())
                        },
                        "Approve new revision"
                    }
                    button {
                        class: "rejection-action",
                        onclick: {
                            let request = GraphRewriteDecisionRequest {
                                proposal_id: proposal.proposal_id.clone(),
                                project_id: proposal.project_id.clone(),
                                decision: GraphRewriteDecision::Reject,
                            };
                            move |_| on_decide.call(request.clone())
                        },
                        "Reject"
                    }
                }
            }
        }
    }
}

fn graph_rewrite_status_label(status: GraphRewriteProposalStatus) -> &'static str {
    match status {
        GraphRewriteProposalStatus::Pending => "Waiting approval",
        GraphRewriteProposalStatus::Approved => "Approved",
        GraphRewriteProposalStatus::Rejected => "Rejected",
    }
}

const GRAPH_EDITOR_SIGNALS: [ControlSignal; 8] = [
    ControlSignal::Succeeded,
    ControlSignal::Failed,
    ControlSignal::Passed,
    ControlSignal::Rejected,
    ControlSignal::NeedsApproval,
    ControlSignal::Approved,
    ControlSignal::BudgetExceeded,
    ControlSignal::Manual,
];

const STANDARD_GRAPH_SIGNALS: [ControlSignal; 7] = [
    ControlSignal::Succeeded,
    ControlSignal::Failed,
    ControlSignal::Passed,
    ControlSignal::Rejected,
    ControlSignal::NeedsApproval,
    ControlSignal::BudgetExceeded,
    ControlSignal::Manual,
];

const APPROVAL_GRAPH_SIGNALS: [ControlSignal; 2] =
    [ControlSignal::Approved, ControlSignal::Rejected];
const AGENT_LOOP_PRIMARY_BRANCH_SIGNALS: [ControlSignal; 2] =
    [ControlSignal::Succeeded, ControlSignal::Failed];
const GATE_PRIMARY_BRANCH_SIGNALS: [ControlSignal; 2] =
    [ControlSignal::Passed, ControlSignal::Rejected];
const AUDIT_PRIMARY_BRANCH_SIGNALS: [ControlSignal; 2] =
    [ControlSignal::Passed, ControlSignal::Failed];

fn branch_signal_options<T: Copy + PartialEq>(
    allowed: &[T],
    preferred: &[T],
    existing: &[T],
    selected: Option<T>,
) -> Vec<T> {
    let mut options = Vec::new();
    for signal in preferred.iter().chain(existing).copied().chain(selected) {
        if allowed.contains(&signal) && !options.contains(&signal) {
            options.push(signal);
        }
    }
    options
}

const GRAPH_INPUT_PORT_Y: f64 = 42.0;
const GRAPH_BRANCH_PORT_START_Y: f64 = 58.0;
const GRAPH_BRANCH_PORT_STEP_Y: f64 = 20.0;
const GRAPH_NODE_MIN_HEIGHT: f64 = 84.0;

fn graph_branch_port_y<T: PartialEq>(signals: &[T], signal: &T) -> f64 {
    let index = signals
        .iter()
        .position(|candidate| candidate == signal)
        .unwrap_or_default();
    GRAPH_BRANCH_PORT_START_Y + branch_index_as_f64(index) * GRAPH_BRANCH_PORT_STEP_Y
}

fn graph_branch_pin_style(index: usize) -> String {
    let top = GRAPH_BRANCH_PORT_START_Y + branch_index_as_f64(index) * GRAPH_BRANCH_PORT_STEP_Y;
    format!("top: {top:.1}px;")
}

fn branch_index_as_f64(index: usize) -> f64 {
    f64::from(u32::try_from(index).unwrap_or(u32::MAX))
}

fn graph_freeform_node_style(point: CanvasPoint, branch_count: usize) -> String {
    let branch_height = if branch_count == 0 {
        GRAPH_NODE_MIN_HEIGHT
    } else {
        GRAPH_BRANCH_PORT_START_Y
            + (branch_index_as_f64(branch_count.saturating_sub(1)) * GRAPH_BRANCH_PORT_STEP_Y)
            + 14.0
    };
    format!(
        "left: {:.1}px; top: {:.1}px; min-height: {:.1}px;",
        point.x,
        point.y,
        branch_height.max(GRAPH_NODE_MIN_HEIGHT),
    )
}

fn control_route_source_port_y(
    graph: &ControlGraphRevision,
    source_node_id: &str,
    signal: ControlSignal,
    selected_signal: ControlSignal,
) -> f64 {
    let Some(node) = graph.nodes.iter().find(|node| node.id == source_node_id) else {
        return GRAPH_INPUT_PORT_Y;
    };
    let existing = graph
        .routes
        .iter()
        .filter(|route| route.source_node_id == source_node_id)
        .map(|route| route.signal)
        .collect::<Vec<_>>();
    let options = branch_signal_options(
        control_signals_for_node(Some(&node.kind)),
        primary_control_branch_signals(&node.kind),
        &existing,
        Some(selected_signal),
    );
    graph_branch_port_y(&options, &signal)
}

fn blueprint_route_source_port_y(
    blueprint: &OrchestrationBlueprintRevision,
    source_node_id: &str,
    signal: ControlSignal,
    selected_signal: ControlSignal,
) -> f64 {
    let Some(node) = blueprint
        .nodes
        .iter()
        .find(|node| node.id == source_node_id)
    else {
        return GRAPH_INPUT_PORT_Y;
    };
    let existing = blueprint
        .links
        .iter()
        .filter_map(|link| (link.source_node_id == source_node_id).then_some(&link.kind))
        .filter_map(|kind| match kind {
            BlueprintLinkKind::Flow { signal } => Some(*signal),
            BlueprintLinkKind::Data { .. } => None,
        })
        .collect::<Vec<_>>();
    let options = branch_signal_options(
        blueprint_signals_for_node(Some(&node.kind)),
        primary_blueprint_branch_signals(&node.kind),
        &existing,
        Some(selected_signal),
    );
    graph_branch_port_y(&options, &signal)
}

fn portfolio_route_source_port_y(
    revision: &PortfolioOrchestrationRevision,
    source_node_id: &str,
    signal: PortfolioSignal,
    selected_signal: Option<PortfolioSignal>,
) -> f64 {
    let Some(node) = revision.nodes.iter().find(|node| node.id == source_node_id) else {
        return GRAPH_INPUT_PORT_Y;
    };
    let allowed = portfolio_allowed_signals(&node.kind);
    let existing = revision
        .routes
        .iter()
        .filter(|route| route.source_node_id == source_node_id)
        .map(|route| route.signal)
        .collect::<Vec<_>>();
    let options = branch_signal_options(
        &allowed,
        primary_portfolio_branch_signals(&node.kind),
        &existing,
        selected_signal,
    );
    graph_branch_port_y(&options, &signal)
}

fn primary_control_branch_signals(kind: &ControlNodeKind) -> &'static [ControlSignal] {
    match kind {
        ControlNodeKind::AgentLoop { .. } => &AGENT_LOOP_PRIMARY_BRANCH_SIGNALS,
        ControlNodeKind::Gate => &GATE_PRIMARY_BRANCH_SIGNALS,
        ControlNodeKind::Audit => &AUDIT_PRIMARY_BRANCH_SIGNALS,
        ControlNodeKind::Approval => &APPROVAL_GRAPH_SIGNALS,
        ControlNodeKind::Terminal => &[],
    }
}

fn primary_blueprint_branch_signals(kind: &BlueprintNodeKind) -> &'static [ControlSignal] {
    match kind {
        BlueprintNodeKind::Approach { .. } => &AGENT_LOOP_PRIMARY_BRANCH_SIGNALS,
        BlueprintNodeKind::Gate => &GATE_PRIMARY_BRANCH_SIGNALS,
        BlueprintNodeKind::Audit => &AUDIT_PRIMARY_BRANCH_SIGNALS,
        BlueprintNodeKind::Approval => &APPROVAL_GRAPH_SIGNALS,
        BlueprintNodeKind::ProjectSelector { .. } | BlueprintNodeKind::PostAction { .. } => {
            &AGENT_LOOP_PRIMARY_BRANCH_SIGNALS
        }
        BlueprintNodeKind::Terminal => &[],
    }
}

fn control_signal_value(signal: ControlSignal) -> &'static str {
    match signal {
        ControlSignal::Succeeded => "succeeded",
        ControlSignal::Failed => "failed",
        ControlSignal::Passed => "passed",
        ControlSignal::Rejected => "rejected",
        ControlSignal::NeedsApproval => "needs_approval",
        ControlSignal::Approved => "approved",
        ControlSignal::BudgetExceeded => "budget_exceeded",
        ControlSignal::Manual => "manual",
    }
}

fn parse_control_signal(value: &str) -> Option<ControlSignal> {
    GRAPH_EDITOR_SIGNALS
        .into_iter()
        .find(|signal| control_signal_value(*signal) == value)
}

fn control_signals_for_node(kind: Option<&ControlNodeKind>) -> &'static [ControlSignal] {
    match kind {
        Some(ControlNodeKind::Approval) => &APPROVAL_GRAPH_SIGNALS,
        Some(ControlNodeKind::Terminal) | None => &[],
        Some(
            ControlNodeKind::AgentLoop { .. } | ControlNodeKind::Gate | ControlNodeKind::Audit,
        ) => &STANDARD_GRAPH_SIGNALS,
    }
}

fn control_node_kind_from_editor(kind: &str, agent_profile_id: &str) -> Option<ControlNodeKind> {
    match kind {
        "agent_loop" if !agent_profile_id.is_empty() => Some(ControlNodeKind::AgentLoop {
            agent_profile_id: agent_profile_id.to_owned(),
        }),
        "gate" => Some(ControlNodeKind::Gate),
        "approval" => Some(ControlNodeKind::Approval),
        "audit" => Some(ControlNodeKind::Audit),
        "terminal" => Some(ControlNodeKind::Terminal),
        _ => None,
    }
}

fn next_canvas_node_id(graph: &ControlGraphRevision, kind: &str) -> String {
    let prefix = match kind {
        "agent_loop" => "agent-loop",
        "gate" => "gate",
        "approval" => "approval",
        "audit" => "audit",
        "terminal" => "terminal",
        _ => "node",
    };
    (1..=graph.nodes.len().saturating_add(1))
        .map(|sequence| format!("{prefix}-{sequence}"))
        .find(|candidate| graph.nodes.iter().all(|node| node.id != *candidate))
        .expect("one identifier in the bounded sequence must be unused")
}

fn control_node_kind_label(kind: &ControlNodeKind) -> String {
    match kind {
        ControlNodeKind::AgentLoop { agent_profile_id } => {
            format!("Agent Loop · {agent_profile_id}")
        }
        ControlNodeKind::Gate => "Gate".to_owned(),
        ControlNodeKind::Approval => "Approval".to_owned(),
        ControlNodeKind::Audit => "Audit".to_owned(),
        ControlNodeKind::Terminal => "Terminal".to_owned(),
    }
}

#[component]
fn ControlGraphPositionCard(
    position: WorkItemGraphPosition,
    graph: Option<ControlGraphRevision>,
    on_evidence_route: EventHandler<EvidenceRouteRequest>,
    on_human_approval: EventHandler<HumanApprovalRequest>,
) -> Element {
    let mut evidence_ref = use_signal(String::new);
    let evidence_ref_value = evidence_ref.read().clone();
    let node = graph.as_ref().and_then(|graph| {
        graph
            .nodes
            .iter()
            .find(|node| node.id == position.current_node_id)
    });
    let routes = graph.as_ref().map_or_else(Vec::new, |graph| {
        graph
            .routes
            .iter()
            .filter(|route| route.source_node_id == position.current_node_id)
            .cloned()
            .collect::<Vec<_>>()
    });
    rsx! {
        article { class: "control-node-position",
            div { class: "control-node-position-heading",
                strong { "{position.work_item_id}" }
                code { "{position.current_node_id}" }
            }
            match node.map(|node| &node.kind) {
                Some(ControlNodeKind::AgentLoop { agent_profile_id }) => rsx! {
                    p { "Agent Loop ready · {agent_profile_id}" }
                },
                Some(kind @ (ControlNodeKind::Gate | ControlNodeKind::Audit)) => {
                    let kind_label = match kind {
                        ControlNodeKind::Gate => "Gate",
                        ControlNodeKind::Audit => "Audit",
                        _ => unreachable!("matched Gate or Audit"),
                    };
                    rsx! {
                        p { "{kind_label} requires an explicit evidence reference." }
                        input {
                            aria_label: "Evidence reference for {position.work_item_id}",
                            value: "{evidence_ref_value}",
                            maxlength: 512,
                            placeholder: "test:cargo-test-workspace",
                            oninput: move |event| evidence_ref.set(event.value()),
                        }
                        div { class: "control-node-actions",
                            for route in &routes {
                                button {
                                    disabled: evidence_ref_value.trim().is_empty(),
                                    onclick: {
                                        let request = EvidenceRouteRequest {
                                            decision_id: next_control_decision_id(),
                                            work_item_id: position.work_item_id.clone(),
                                            expected_current_node_id: position.current_node_id.clone(),
                                            route_id: route.id.clone(),
                                            evidence_refs: vec![evidence_ref_value.trim().to_owned()],
                                        };
                                        move |_| on_evidence_route.call(request.clone())
                                    },
                                    "Record {control_signal_label(route.signal)}"
                                }
                            }
                        }
                    }
                },
                Some(ControlNodeKind::Approval) => rsx! {
                    details { class: "control-node-approval",
                        summary { "Review human approval" }
                        p { "This action records an immutable human decision and advances only the selected declared route." }
                        div { class: "control-node-actions",
                            for route in &routes {
                                button {
                                    class: if route.signal == ControlSignal::Approved { "approval-action" } else { "rejection-action" },
                                    onclick: {
                                        let decision_id = next_control_decision_id();
                                        let request = HumanApprovalRequest {
                                            decision_id: decision_id.clone(),
                                            work_item_id: position.work_item_id.clone(),
                                            expected_current_node_id: position.current_node_id.clone(),
                                            route_id: route.id.clone(),
                                            evidence_refs: vec![format!("human:desktop:{decision_id}")],
                                        };
                                        move |_| on_human_approval.call(request.clone())
                                    },
                                    "{control_signal_label(route.signal)} and continue"
                                }
                            }
                        }
                    }
                },
                Some(ControlNodeKind::Terminal) => rsx! {
                    p { class: "control-node-terminal", "Graph complete" }
                },
                None => rsx! {
                    p { class: "field-warning", "Pinned Control node is unavailable." }
                },
            }
        }
    }
}

fn control_signal_label(signal: ControlSignal) -> &'static str {
    match signal {
        ControlSignal::Succeeded => "Succeeded",
        ControlSignal::Failed => "Failed",
        ControlSignal::Passed => "Passed",
        ControlSignal::Rejected => "Rejected",
        ControlSignal::NeedsApproval => "Needs approval",
        ControlSignal::Approved => "Approve",
        ControlSignal::BudgetExceeded => "Budget exceeded",
        ControlSignal::Manual => "Manual",
    }
}

fn outcome_class(outcome: CheckpointOutcome) -> &'static str {
    match outcome {
        CheckpointOutcome::Progress
        | CheckpointOutcome::Completed
        | CheckpointOutcome::NoAction => "outcome outcome-positive",
        CheckpointOutcome::NeedsReview => "outcome outcome-review",
        CheckpointOutcome::Blocked | CheckpointOutcome::Failed => "outcome outcome-attention",
    }
}

fn delivery_class(status: CheckpointDeliveryStatus) -> &'static str {
    match status {
        CheckpointDeliveryStatus::Synced => "delivery delivery-synced",
        CheckpointDeliveryStatus::Pending => "delivery delivery-pending",
        CheckpointDeliveryStatus::Conflict | CheckpointDeliveryStatus::Failed => {
            "delivery delivery-attention"
        }
    }
}

fn display_time(recorded_at: &str) -> String {
    recorded_at.split_once('T').map_or_else(
        || recorded_at.to_owned(),
        |(date, time)| {
            let short_time: String = time.chars().take(5).collect();
            format!("{date} {short_time}")
        },
    )
}

fn health_class(health: ProjectHealth) -> &'static str {
    match health {
        ProjectHealth::Healthy => "health health-healthy",
        ProjectHealth::Blocked => "health health-blocked",
        ProjectHealth::Idle => "health health-idle",
    }
}

#[component]
fn Metric(value: String, label: &'static str) -> Element {
    rsx! {
        article { class: "metric",
            strong { "{value}" }
            span { "{label}" }
        }
    }
}

#[component]
fn ProjectStat(label: &'static str, value: u32) -> Element {
    rsx! {
        div {
            strong { "{value}" }
            span { "{label}" }
        }
    }
}

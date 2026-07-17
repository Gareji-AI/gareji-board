use std::env;
use std::path::PathBuf;

use dioxus::prelude::*;
use gareji_board_core::CoreProgressReader;
use gareji_board_domain::{
    ActivityTimeline, AgentPlan, AgentPlanUpdateReceipt, AgentPlanUpdateRequest,
    AgentProfileSummary, ApprovalRequirement, AttachmentReceipt, AttachmentRequest,
    AttachmentTarget, AutopilotStopReason, CandidateSkipReason, CheckpointDeliveryStatus,
    CheckpointOutcome, CheckpointReconciliation, NoCandidateReason, PortfolioSnapshot,
    ProgressActivity, ProjectHealth, ReconciliationDecision, ReconciliationReceipt,
    ReconciliationRequest, SafeAutopilotOutcome, SafeAutopilotPreview, WorkItemState,
    WorkItemSummary, WorkItemTransitionReceipt, WorkItemTransitionRequest,
};
use gareji_board_store::{SqliteBoardStore, default_board_database_path};

const APP_CSS: &str = include_str!("style.css");
const PREVIEW_GLOBAL_CONCURRENCY_CAP: u32 = 2;
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
    dioxus::launch(App);
}

#[derive(Clone)]
struct AppState {
    portfolio: PortfolioSnapshot,
    work_items: Vec<WorkItemSummary>,
    agent_profiles: Vec<AgentProfileSummary>,
    activity: ActivityTimeline,
    storage_label: String,
    warning: Option<String>,
    activity_warning: Option<String>,
    reconciliation_notice: Option<String>,
    attachment_notice: Option<String>,
    transition_notice: Option<String>,
    agent_plan_notice: Option<String>,
    autopilot_preview: Option<SafeAutopilotPreview>,
}

fn load_app_state() -> AppState {
    let database_path = board_database_path();
    let storage_label = database_path.display().to_string();
    let loaded = SqliteBoardStore::open(&database_path).and_then(|mut store| {
        store.seed_sample_if_empty()?;
        let portfolio = store.load_portfolio()?;
        let work_items = store.load_work_items()?;
        let agent_profiles = store.load_agent_profiles()?;
        Ok((store, portfolio, work_items, agent_profiles))
    });

    match loaded {
        Ok((store, portfolio, work_items, agent_profiles)) => {
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
                activity,
                storage_label,
                warning: None,
                activity_warning,
                reconciliation_notice: None,
                attachment_notice: None,
                transition_notice: None,
                agent_plan_notice: None,
                autopilot_preview: None,
            }
        }
        Err(error) => AppState {
            portfolio: PortfolioSnapshot::default(),
            work_items: Vec::new(),
            agent_profiles: Vec::new(),
            activity: ActivityTimeline::default(),
            storage_label,
            warning: Some(error.to_string()),
            activity_warning: None,
            reconciliation_notice: None,
            attachment_notice: None,
            transition_notice: None,
            agent_plan_notice: None,
            autopilot_preview: None,
        },
    }
}

fn board_database_path() -> PathBuf {
    env::var_os("GAREJI_BOARD_DB")
        .filter(|value| !value.is_empty())
        .map_or_else(default_board_database_path, PathBuf::from)
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

#[allow(non_snake_case)]
fn App() -> Element {
    let mut state = use_signal(load_app_state);
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

    rsx! {
        document::Title { "Gareji Board" }
        style { {APP_CSS} }
        main { class: "shell",
            header { class: "topbar",
                div {
                    p { class: "eyebrow", "LOCAL AUTOPILOT CONTROL" }
                    h1 { "Gareji Board" }
                }
                div { class: "status-pill", span { class: "status-dot" } "Local · Ready" }
            }

            PortfolioHero { on_preview }

            section { class: "metrics", aria_label: "Portfolio metrics",
                Metric { value: project_count.to_string(), label: "Projects" }
                Metric { value: active_runs.to_string(), label: "Active runs" }
                Metric { value: blocked_items.to_string(), label: "Need attention" }
                Metric { value: delivery_issues.to_string(), label: "Delivery issues" }
            }

            if let Some(preview) = &snapshot.autopilot_preview {
                AutopilotPreviewPanel { preview: preview.clone() }
            }

            if let Some(warning) = &snapshot.warning {
                aside { class: "warning", "Local data could not be loaded: {warning}" }
            }

            if let Some(notice) = &snapshot.reconciliation_notice {
                aside { class: "action-notice", "{notice}" }
            }

            if let Some(notice) = &snapshot.attachment_notice {
                aside { class: "action-notice", "{notice}" }
            }

            if let Some(notice) = &snapshot.transition_notice {
                aside { class: "action-notice", "{notice}" }
            }

            if let Some(notice) = &snapshot.agent_plan_notice {
                aside { class: "action-notice", "{notice}" }
            }

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

            WorkItemControl {
                work_items: snapshot.work_items.clone(),
                agent_profiles: snapshot.agent_profiles.clone(),
                on_transition,
                on_agent_plan,
            }

            ProjectGrid { portfolio: snapshot.portfolio.clone() }

            footer { class: "app-footer",
                span { "Local data" }
                code { "{snapshot.storage_label}" }
            }
        }
    }
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
fn WorkItemControl(
    work_items: Vec<WorkItemSummary>,
    agent_profiles: Vec<AgentProfileSummary>,
    on_transition: EventHandler<WorkItemTransitionRequest>,
    on_agent_plan: EventHandler<AgentPlanUpdateRequest>,
) -> Element {
    let work_item_count = work_items.len();
    let capability_catalog = agent_capability_catalog(&agent_profiles, &work_items);
    let lanes = group_work_items_by_state(work_items);
    rsx! {
        section { class: "section-heading",
            div {
                p { class: "kicker", "State authority" }
                h3 { "Work item Kanban" }
            }
            span { "{work_item_count} total" }
        }

        if work_item_count == 0 {
            section { class: "empty-work-items",
                strong { "No Work items yet" }
                p { "Create one from an Activity Inbox Checkpoint to begin." }
            }
        } else {
            section { class: "kanban-board", aria_label: "Work item Kanban board",
                for (state, lane_items) in lanes {
                    KanbanLane {
                        key: "{state.as_str()}",
                        state,
                        work_items: lane_items,
                        agent_profiles: agent_profiles.clone(),
                        capability_catalog: capability_catalog.clone(),
                        on_transition,
                        on_agent_plan,
                    }
                }
            }
        }
    }
}

#[component]
fn KanbanLane(
    state: WorkItemState,
    work_items: Vec<WorkItemSummary>,
    agent_profiles: Vec<AgentProfileSummary>,
    capability_catalog: Vec<String>,
    on_transition: EventHandler<WorkItemTransitionRequest>,
    on_agent_plan: EventHandler<AgentPlanUpdateRequest>,
) -> Element {
    let item_count = work_items.len();
    let state_label = work_item_state_label(state);
    let item_noun = if item_count == 1 { "item" } else { "items" };
    rsx! {
        section {
            class: kanban_lane_class(state),
            aria_label: "{state_label} lane, {item_count} {item_noun}",
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
                            agent_profiles: agent_profiles.clone(),
                            capability_catalog: capability_catalog.clone(),
                            on_transition,
                            on_agent_plan,
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
    agent_profiles: Vec<AgentProfileSummary>,
    capability_catalog: Vec<String>,
    on_transition: EventHandler<WorkItemTransitionRequest>,
    on_agent_plan: EventHandler<AgentPlanUpdateRequest>,
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
        article { class: "work-item-card",
            div { class: "work-item-head",
                div {
                    p { class: "work-item-id", "{item.id}" }
                    h4 { "{item.title}" }
                }
                span { class: work_item_state_class(item.state),
                    "{work_item_state_label(item.state)}"
                }
            }
            p { class: "work-item-project", "{item.project_id} · Priority {item.priority}" }
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
                        onclick: move |_| on_agent_plan.call(agent_plan_request.clone()),
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
                    onclick: move |_| on_transition.call(request.clone()),
                    "Update state"
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
fn AutopilotPreviewPanel(preview: SafeAutopilotPreview) -> Element {
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
                    span { class: "preview-read-only", "Read-only · Runner not started" }
                }
            }

            match &preview.outcome {
                SafeAutopilotOutcome::Candidate(candidate) => rsx! {
                    article { class: "preview-candidate",
                        div { class: "preview-candidate-id",
                            span { "Selected candidate" }
                            strong { "{candidate.work_item.id}" }
                        }
                        div { class: "preview-candidate-body",
                            h4 { "{candidate.work_item.title}" }
                            p {
                                "{candidate.project_name} is at {candidate.active_runs}/{candidate.execution_cap} active capacity. {candidate.agent_profile.role} is assigned and satisfies every required Agent capability. This item is priority {candidate.work_item.priority}."
                            }
                            div { class: "preview-facts",
                                span { "Board gates passed" }
                                span { "Lowest project load first" }
                                span { "Then priority" }
                                span { "Then stable IDs" }
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

#[cfg(test)]
mod tests {
    use super::*;

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
            },
            AgentProfileSummary {
                id: "implementer".to_owned(),
                role: "Implementer".to_owned(),
                capabilities: vec!["implementation".to_owned(), "testing".to_owned()],
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
}

#[component]
fn ProjectGrid(portfolio: PortfolioSnapshot) -> Element {
    let project_count = portfolio.projects.len();
    rsx! {
        section { class: "section-heading",
            div {
                p { class: "kicker", "Connected work" }
                h3 { "Projects" }
            }
            span { "{project_count} total" }
        }

        section { class: "project-grid",
            for project in &portfolio.projects {
                article { class: "project-card", key: "{project.id}",
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
                    footer {
                        span { "Concurrency {project.work_items.in_progress}/{project.execution_cap}" }
                        span { "{project.work_items.total} work items" }
                    }
                }
            }
        }
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

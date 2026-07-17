use std::env;
use std::path::PathBuf;

use dioxus::prelude::*;
use gareji_board_core::CoreProgressReader;
use gareji_board_domain::{
    ActivityTimeline, CheckpointDeliveryStatus, CheckpointOutcome, CheckpointReconciliation,
    PortfolioSnapshot, ProjectHealth, ReconciliationDecision, ReconciliationReceipt,
    ReconciliationRequest, WorkItemState,
};
use gareji_board_store::{SqliteBoardStore, default_board_database_path};

const APP_CSS: &str = include_str!("style.css");

fn main() {
    dioxus::launch(App);
}

#[derive(Clone)]
struct AppState {
    portfolio: PortfolioSnapshot,
    activity: ActivityTimeline,
    storage_label: String,
    warning: Option<String>,
    activity_warning: Option<String>,
    reconciliation_notice: Option<String>,
}

fn load_app_state() -> AppState {
    let database_path = board_database_path();
    let storage_label = database_path.display().to_string();
    let loaded = SqliteBoardStore::open(&database_path).and_then(|mut store| {
        store.seed_sample_if_empty()?;
        let portfolio = store.load_portfolio()?;
        Ok((store, portfolio))
    });

    match loaded {
        Ok((store, portfolio)) => {
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
            if let Err(error) = store.hydrate_activity_reconciliations(&mut activity) {
                activity_warning = Some(format!("Reconciliation history: {error}"));
            }
            AppState {
                portfolio,
                activity,
                storage_label,
                warning: None,
                activity_warning,
                reconciliation_notice: None,
            }
        }
        Err(error) => AppState {
            portfolio: PortfolioSnapshot::default(),
            activity: ActivityTimeline::default(),
            storage_label,
            warning: Some(error.to_string()),
            activity_warning: None,
            reconciliation_notice: None,
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

#[allow(non_snake_case)]
fn App() -> Element {
    let mut state = use_signal(load_app_state);
    let snapshot = state.read().clone();
    let project_count = snapshot.portfolio.projects.len();
    let active_runs = snapshot.portfolio.active_runs();
    let blocked_items = snapshot.portfolio.blocked_items();
    let delivery_issues = snapshot.activity.delivery_issues();
    let on_reconcile = move |request: ReconciliationRequest| match reconcile(&request) {
        Ok(receipt) => {
            let mut reloaded = load_app_state();
            reloaded.reconciliation_notice = Some(reconciliation_message(&receipt));
            state.set(reloaded);
        }
        Err(error) => state.write().reconciliation_notice = Some(error),
    };

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

            section { class: "hero",
                div {
                    p { class: "kicker", "Portfolio overview" }
                    h2 { "See what every agent is doing—without opening a terminal." }
                    p { class: "lede", "Board-owned coordination stays local. Recent progress is read from Core as evidence, while Work item changes remain yours to approve." }
                }
                button { class: "primary", disabled: true, "Run next safe item" }
            }

            section { class: "metrics", aria_label: "Portfolio metrics",
                Metric { value: project_count.to_string(), label: "Projects" }
                Metric { value: active_runs.to_string(), label: "Active runs" }
                Metric { value: blocked_items.to_string(), label: "Need attention" }
                Metric { value: delivery_issues.to_string(), label: "Delivery issues" }
            }

            if let Some(warning) = &snapshot.warning {
                aside { class: "warning", "Local data could not be loaded: {warning}" }
            }

            if let Some(notice) = &snapshot.reconciliation_notice {
                aside { class: "action-notice", "{notice}" }
            }

            RecentActivity {
                activity: snapshot.activity.clone(),
                warning: snapshot.activity_warning.clone(),
                on_reconcile,
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
fn RecentActivity(
    activity: ActivityTimeline,
    warning: Option<String>,
    on_reconcile: EventHandler<ReconciliationRequest>,
) -> Element {
    rsx! {
        section { class: "section-heading",
            div {
                p { class: "kicker", "Evidence from Core" }
                h3 { "Recent activity" }
            }
            span { "{activity.activities.len()} checkpoints" }
        }

        if let Some(warning) = &warning {
            aside { class: "notice", "Progress history: {warning}" }
        }

        if activity.activities.is_empty() {
            section { class: "empty-activity",
                strong { "No progress checkpoints yet" }
                p { "Connect this Board to Gareji Core, then record work from Runner, MCP, or direct development." }
            }
        } else {
            section { class: "timeline", aria_label: "Recent progress checkpoints",
                for item in &activity.activities {
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
                                if let Some(work_item_id) = &item.work_item_id {
                                    span { " · {work_item_id}" }
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
                                work_item_id: item.work_item_id.clone(),
                                recommended_state: item.recommended_state,
                                reconciliation: item.reconciliation.clone(),
                                on_reconcile,
                            }
                        }
                    }
                }
            }
            if activity.has_older {
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

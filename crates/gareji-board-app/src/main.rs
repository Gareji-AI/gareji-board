use dioxus::prelude::*;
use gareji_board_core::CoreProgressReader;
use gareji_board_domain::{
    ActivityTimeline, CheckpointDeliveryStatus, CheckpointOutcome, PortfolioSnapshot, ProjectHealth,
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
}

fn load_app_state() -> AppState {
    let database_path = default_board_database_path();
    let storage_label = database_path.display().to_string();
    let loaded = SqliteBoardStore::open(&database_path).and_then(|mut store| {
        store.seed_sample_if_empty()?;
        store.load_portfolio()
    });

    match loaded {
        Ok(portfolio) => {
            let project_ids = portfolio
                .projects
                .iter()
                .map(|project| project.id.clone())
                .collect::<Vec<_>>();
            let (activity, activity_warning) = CoreProgressReader::from_environment()
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
            AppState {
                portfolio,
                activity,
                storage_label,
                warning: None,
                activity_warning,
            }
        }
        Err(error) => AppState {
            portfolio: PortfolioSnapshot::default(),
            activity: ActivityTimeline::default(),
            storage_label,
            warning: Some(error.to_string()),
            activity_warning: None,
        },
    }
}

#[allow(non_snake_case)]
fn App() -> Element {
    let state = use_hook(load_app_state);
    let project_count = state.portfolio.projects.len();
    let active_runs = state.portfolio.active_runs();
    let blocked_items = state.portfolio.blocked_items();
    let delivery_issues = state.activity.delivery_issues();

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

            if let Some(warning) = &state.warning {
                aside { class: "warning", "Local data could not be loaded: {warning}" }
            }

            RecentActivity {
                activity: state.activity.clone(),
                warning: state.activity_warning.clone(),
            }

            ProjectGrid { portfolio: state.portfolio.clone() }

            footer { class: "app-footer",
                span { "Local data" }
                code { "{state.storage_label}" }
            }
        }
    }
}

#[component]
fn RecentActivity(activity: ActivityTimeline, warning: Option<String>) -> Element {
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

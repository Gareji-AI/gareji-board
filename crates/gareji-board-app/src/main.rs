use std::path::PathBuf;

use dioxus::prelude::*;
use directories::ProjectDirs;
use gareji_board_domain::{PortfolioSnapshot, ProjectHealth};
use gareji_board_store::SqliteBoardStore;

const APP_CSS: &str = include_str!("style.css");

fn main() {
    dioxus::launch(App);
}

#[derive(Clone)]
struct AppState {
    portfolio: PortfolioSnapshot,
    storage_label: String,
    warning: Option<String>,
}

fn load_app_state() -> AppState {
    let database_path = board_database_path();
    let storage_label = database_path.display().to_string();
    let loaded = SqliteBoardStore::open(&database_path).and_then(|mut store| {
        store.seed_sample_if_empty()?;
        store.load_portfolio()
    });

    match loaded {
        Ok(portfolio) => AppState {
            portfolio,
            storage_label,
            warning: None,
        },
        Err(error) => AppState {
            portfolio: PortfolioSnapshot::default(),
            storage_label,
            warning: Some(error.to_string()),
        },
    }
}

fn board_database_path() -> PathBuf {
    ProjectDirs::from("dev", "Gareji", "Gareji Board").map_or_else(
        || PathBuf::from("gareji-board.sqlite3"),
        |directories| directories.data_local_dir().join("board.sqlite3"),
    )
}

#[allow(non_snake_case)]
fn App() -> Element {
    let state = use_hook(load_app_state);
    let project_count = state.portfolio.projects.len();
    let active_runs = state.portfolio.active_runs();
    let blocked_items = state.portfolio.blocked_items();

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
                    p { class: "lede", "This first Rust screen reads Board-owned coordination state from local SQLite. Execution remains disabled until Core and Runner are connected." }
                }
                button { class: "primary", disabled: true, "Run next safe item" }
            }

            section { class: "metrics", aria_label: "Portfolio metrics",
                Metric { value: project_count.to_string(), label: "Projects" }
                Metric { value: active_runs.to_string(), label: "Active runs" }
                Metric { value: blocked_items.to_string(), label: "Need attention" }
            }

            if let Some(warning) = &state.warning {
                aside { class: "warning", "Local data could not be loaded: {warning}" }
            }

            section { class: "section-heading",
                div {
                    p { class: "kicker", "Connected work" }
                    h3 { "Projects" }
                }
                span { "{project_count} total" }
            }

            section { class: "project-grid",
                for project in &state.portfolio.projects {
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

            footer { class: "app-footer",
                span { "Local data" }
                code { "{state.storage_label}" }
            }
        }
    }
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

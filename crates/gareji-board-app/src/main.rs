#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// Tauri owns deserialized command arguments. Keeping them by value makes the IPC
// boundary explicit even though the Board Module only needs to borrow them.
#![allow(clippy::needless_pass_by_value)]

mod board_module;

use board_module::{
    ActionResult, AttachActivityRequest, BindGraphRequest, BoardModule, CreateProjectRequest,
    CreateWorkItemRequest, DesktopSnapshot, PreviewResult, ReconcileActivityRequest,
    SaveAgentProfileRequest, StartAgentLoopRequest, TransitionRequest, UpdateAgentPlanRequest,
};
use gareji_board_domain::{
    ControlGraphRevision, GraphCanvasLayout, OrchestrationBlueprintRevision,
};

#[tauri::command]
fn load_desktop() -> Result<DesktopSnapshot, String> {
    BoardModule::load()
}

#[tauri::command]
fn preview_safe_autopilot() -> Result<PreviewResult, String> {
    BoardModule::preview_safe_autopilot()
}

#[tauri::command]
fn transition_work_item(request: TransitionRequest) -> Result<ActionResult, String> {
    BoardModule::transition_work_item(&request)
}

#[tauri::command]
fn create_work_item(request: CreateWorkItemRequest) -> Result<ActionResult, String> {
    BoardModule::create_work_item(&request)
}

#[tauri::command]
fn create_project(request: CreateProjectRequest) -> Result<ActionResult, String> {
    BoardModule::create_project(&request)
}

#[tauri::command]
fn save_graph_layout(layout: GraphCanvasLayout) -> Result<ActionResult, String> {
    BoardModule::save_graph_layout(&layout)
}

#[tauri::command]
fn publish_graph_revision(graph: ControlGraphRevision) -> Result<ActionResult, String> {
    BoardModule::publish_graph_revision(&graph)
}

#[tauri::command]
fn bind_project_graph(request: BindGraphRequest) -> Result<ActionResult, String> {
    BoardModule::bind_project_graph(&request)
}

#[tauri::command]
fn publish_blueprint_revision(
    blueprint: OrchestrationBlueprintRevision,
) -> Result<ActionResult, String> {
    BoardModule::publish_blueprint_revision(&blueprint)
}

#[tauri::command]
fn update_agent_plan(request: UpdateAgentPlanRequest) -> Result<ActionResult, String> {
    BoardModule::update_agent_plan(&request)
}

#[tauri::command]
fn save_agent_profile(request: SaveAgentProfileRequest) -> Result<ActionResult, String> {
    BoardModule::save_agent_profile(&request)
}

#[tauri::command]
fn reconcile_activity(request: ReconcileActivityRequest) -> Result<ActionResult, String> {
    BoardModule::reconcile_activity(&request)
}

#[tauri::command]
fn attach_activity(request: AttachActivityRequest) -> Result<ActionResult, String> {
    BoardModule::attach_activity(&request)
}

#[tauri::command]
async fn start_agent_loop(request: StartAgentLoopRequest) -> Result<ActionResult, String> {
    tauri::async_runtime::spawn_blocking(move || BoardModule::start_agent_loop(&request))
        .await
        .map_err(|error| format!("Runner task could not be joined: {error}"))?
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            load_desktop,
            preview_safe_autopilot,
            transition_work_item,
            create_work_item,
            create_project,
            save_graph_layout,
            publish_graph_revision,
            bind_project_graph,
            publish_blueprint_revision,
            update_agent_plan,
            save_agent_profile,
            reconcile_activity,
            attach_activity,
            start_agent_loop,
        ])
        .run(tauri::generate_context!())
        .expect("Gareji Board could not start its Tauri host");
}

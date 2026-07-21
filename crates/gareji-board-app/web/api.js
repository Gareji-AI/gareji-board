const invoke = window.__TAURI__?.core?.invoke;

async function call(command, payload = {}) {
  if (!invoke) {
    throw new Error("The Gareji Board Tauri host is unavailable.");
  }
  return invoke(command, payload);
}

export const board = {
  load: () => call("load_desktop"),
  previewAutopilot: () => call("preview_safe_autopilot"),
  transitionWorkItem: (request) => call("transition_work_item", { request }),
  createWorkItem: (request) => call("create_work_item", { request }),
  createProject: (request) => call("create_project", { request }),
  saveGraphLayout: (layout) => call("save_graph_layout", { layout }),
  publishGraph: (graph) => call("publish_graph_revision", { graph }),
  bindProjectGraph: (request) => call("bind_project_graph", { request }),
  publishBlueprint: (blueprint) => call("publish_blueprint_revision", { blueprint }),
  updateAgentPlan: (request) => call("update_agent_plan", { request }),
  saveAgentProfile: (request) => call("save_agent_profile", { request }),
  reconcileActivity: (request) => call("reconcile_activity", { request }),
  attachActivity: (request) => call("attach_activity", { request }),
  startAgentLoop: (request) => call("start_agent_loop", { request }),
};
